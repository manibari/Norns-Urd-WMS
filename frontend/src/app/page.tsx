"use client";

/**
 * Receiving — mirrors the paper acceptance form (包材驗收單).
 *
 * Field order follows the form: 進貨日期 / 廠商名稱 / 原物料名稱 / 型號 / 數量(米).
 * Someone is transcribing a sheet of paper; reordering the fields costs them
 * their place on it every single line.
 *
 * 型號 (T6050BSW) is what they write and what the metres table is keyed on. The
 * long code on the box (2003.T7320BC-340X900-P1) is master data they never
 * type — it exists so recognition can map a label back to a 型號.
 *
 * This screen is the whole reason Urd-WMS can run without an ERP: the FIFO sort
 * key (receipt_date) is entered here, by the person holding the box, at the
 * moment they stamp it. Nothing downstream can be more accurate than this.
 *
 * Every field is free-form, including the item code: receiving is also when a
 * new film first shows up at the factory, and sending the warehouse to a
 * separate master-data screen first is how paper survives.
 */

import { ArrowRightOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Checkbox, Col, DatePicker, Divider, Empty, Form, Input,
  InputNumber, Modal, Popconfirm, Radio, Row, Segmented, Select, Space, Table, Tag,
  Tooltip, Typography, message,
} from "antd";
import dayjs from "dayjs";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import CreatableSelect from "@/components/CreatableSelect";
import { api, type Dictionary, type Item, type Lot } from "@/lib/api";

const { Title, Text } = Typography;

// The form's 檢驗項目 block. 規格尺寸 is a tick ("matches spec"), not a value —
// only unusual items get an actual dimension written in, which is why this is a
// checklist rather than five text fields.
const INSPECTION = ["規格尺寸", "標示製造日期", "標示有效日期", "外觀", "顏色"] as const;

export default function ReceivingPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [lots, setLots] = useState<Lot[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [options, setOptions] = useState<{ supplier: string[]; material_name: string[]; spec: string[] }>(
    { supplier: [], material_name: [], spec: [] },
  );
  const [busy, setBusy] = useState(false);
  const [enteredCount, setEnteredCount] = useState(0);
  const [editing, setEditing] = useState<Lot | null>(null);
  const { user, can } = useAuth();
  const [roster, setRoster] = useState<{ name: string; title: string | null; role_label: string }[]>([]);
  useEffect(() => {
    api.signers().then(setRoster).catch(() => undefined);
  }, []);
  const [editForm] = Form.useForm();
  const [knownCode, setKnownCode] = useState(false);
  // Metres is the default: the acceptance form records quantity in metres, so
  // boxes is the exception here, not the norm.
  const [qtyMode, setQtyMode] = useState<"米" | "箱">("米");
  const [form] = Form.useForm();
  const rate = Form.useWatch("meters_per_box", form) as number | undefined;
  const metres = Form.useWatch("qty_meters", form) as number | undefined;
  const chosenCode = (Form.useWatch("item_code", form) as string | undefined)?.trim();
  const verdict = Form.useWatch("verdict", form) as string | undefined;
  const confirmedBy = (Form.useWatch("confirmed_by", form) as string | undefined)?.trim();
  const sameSigner = Boolean(confirmedBy && confirmedBy === user?.name);

  // Preview only — the server does the authoritative conversion. Shown live so
  // the remainder is visible before submitting, not discovered afterwards.
  const preview =
    qtyMode === "米" && rate && rate > 0 && metres && metres > 0
      ? { boxes: Math.floor(metres / rate), remainder: metres % rate }
      : null;

  const load = useCallback(async () => {
    // Settled, not all: a single failing endpoint must not blank the page into
    // looking like an empty warehouse.
    const [i, l, d, o] = await Promise.allSettled([
      api.items(), api.lots(), api.dictionary(), api.itemOptions(),
    ]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`品項載入失敗：${i.reason?.message ?? i.reason}`);
    if (l.status === "fulfilled") setLots(l.value);
    else message.error(`批次載入失敗：${l.reason?.message ?? l.reason}`);
    if (d.status === "fulfilled") setDict(d.value);
    else message.error(`選項載入失敗：${d.reason?.message ?? d.reason}`);
    if (o.status === "fulfilled") setOptions(o.value);
  }, []);

  useEffect(() => { load(); }, [load]);

  // 廠商 / 原物料名稱 / 規格 are attributes of a 型號, so their suggestions are
  // the distinct values already on the item master. A separate dictionary would
  // be the same facts stored twice, free to drift apart. Only needed while
  // defining a NEW 型號 — for a known one these fields are read-only.
  const suppliers = options.supplier;
  const materialNames = options.material_name;
  const specs = options.spec;
  const byCode = new Map(items.map((i) => [i.item_code, i]));

  // A 型號 IS a supplier's material at a spec — T7320BC is 臺灣希悅爾's
  // 高阻氧拉伸膜 340mm x 900M. Letting someone pick those three separately allows
  // combinations that do not exist, so choosing the 型號 fills them in and they
  // become read-only. They are only editable while defining a NEW 型號.
  function onCodeChange(value: string) {
    const found = items.find((i) => i.item_code === (value ?? "").trim());
    setKnownCode(Boolean(found));
    if (found) {
      form.setFieldsValue({
        item_name: found.name,
        spec: found.spec,
        shelf_life_days: found.shelf_life_days,
        safety_stock: found.safety_stock,
        meters_per_box: found.meters_per_box,
        supplier_code: found.supplier_code,
        supplier: found.supplier,
      });
    } else {
      form.setFieldsValue({
        item_name: undefined, spec: undefined, supplier: undefined,
        meters_per_box: undefined, supplier_code: undefined,
      });
    }
  }

  async function submit() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      const res = await api.createLot({
        item_code: values.item_code.trim(),
        item_name: values.item_name,
        spec: values.spec || null,
        shelf_life_days: values.shelf_life_days ?? null,
        safety_stock: values.safety_stock ?? 0,
        receipt_date: dayjs(values.receipt_date).format("YYYY-MM-DD"),
        manufacture_date: values.manufacture_date ? dayjs(values.manufacture_date).format("YYYY-MM-DD") : null,
        supplier: values.supplier || null,
        expiry_date: values.expiry_date ? dayjs(values.expiry_date).format("YYYY-MM-DD") : null,
        entered_unit: qtyMode,
        inspection: Object.fromEntries(
          INSPECTION.map((k) => [k, (values.inspection ?? []).includes(k)]),
        ),
        verdict: values.verdict ?? null,
        confirmed_by: values.confirmed_by || null,
        remark: values.remark || null,
        supplier_lot_code: values.supplier_lot_code || null,
        supplier_code: values.supplier_code || null,
        meters_per_box: values.meters_per_box ?? null,
        ...(qtyMode === "米" ? { qty_meters: values.qty_meters } : { qty: values.qty }),
      });
      message.success(
        `${res.created_item ? "已新增品項並建立批次" : "已建立批次"}：${res.qty} 箱（進貨日 ${res.receipt_date}）`,
      );
      if (res.conversion_note) message.warning(res.conversion_note, 8);
      if (res.verdict === "不合格") {
        message.warning("判定不合格：已留紀錄，但不計入可領用庫存，也不會被 FIFO 指到。", 8);
      }
      if (res.same_signer) {
        message.warning("記錄人與確認人是同一個人 —— 雙簽的意義就是兩個人，請確認。", 8);
      }
      if (res.duplicate_lot_exists) {
        message.warning("同型號、同進貨日、同製造日已有一批 —— 確認不是重複登錄。", 8);
      }
      setEnteredCount((n) => n + 1);
      // Continuous entry: the header fields (進貨日期 / 廠商 / 簽核人) repeat down
      // the sheet — on paper they are literally written as 〃. Clearing them
      // every line would make transcribing one delivery of five batches five
      // times the typing.
      form.resetFields([
        "item_code", "item_name", "spec", "qty", "qty_meters",
        "manufacture_date", "expiry_date", "supplier_lot_code",
        "inspection", "verdict", "remark", "meters_per_box", "supplier_code",
        "shelf_life_days", "safety_stock",
      ]);
      setKnownCode(false);
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function saveEdit() {
    const values = await editForm.validateFields().catch(() => null);
    if (!values || !editing) return;
    setBusy(true);
    try {
      const res = await api.patchLot(editing.id, {
        receipt_date: values.receipt_date ? dayjs(values.receipt_date).format("YYYY-MM-DD") : undefined,
        manufacture_date: values.manufacture_date ? dayjs(values.manufacture_date).format("YYYY-MM-DD") : undefined,
        expiry_date: values.expiry_date ? dayjs(values.expiry_date).format("YYYY-MM-DD") : undefined,
        supplier: values.supplier || undefined,
        supplier_lot_code: values.supplier_lot_code || undefined,
        qty_on_hand: values.qty_on_hand,
        verdict: values.verdict || undefined,
        remark: values.remark || undefined,
      });
      message.success("已更新，變更前後值已寫入異動軌跡");
      if (res.posted_draws > 0) {
        message.warning(`這批已有 ${res.posted_draws} 筆領用紀錄，改動會影響既有帳。`, 8);
      }
      setEditing(null);
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function removeLot(lot: Lot) {
    try {
      await api.deleteLot(lot.id);
      message.success("已刪除");
      load();
    } catch (e) {
      message.error((e as Error).message, 10);
    }
  }

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>收貨建批</Title>

      <Card
        title="登錄一批到貨"
        extra={
          enteredCount > 0 ? (
            <Space>
              <Text type="secondary">本次已登錄 {enteredCount} 批</Text>
              <Button
                size="small"
                onClick={() => {
                  form.resetFields();
                  form.setFieldsValue({ qty: 1, receipt_date: dayjs(), safety_stock: 0 });
                  setEnteredCount(0);
                  setKnownCode(false);
                }}
              >
                清空重填
              </Button>
            </Space>
          ) : null
        }
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{ qty: 1, receipt_date: dayjs(), safety_stock: 0 }}
        >
          <Row gutter={24}>
            <Col xs={24} md={6}>
              <Form.Item
                name="receipt_date"
                label="進貨日期"
                rules={[{ required: true, message: "請填進貨日期" }]}
                extra={enteredCount > 0 ? "沿用上一批（單上的〃）" : "FIFO 就是照這個排序"}
              >
                <DatePicker style={{ width: "100%" }} disabledDate={(d) => d.isAfter(dayjs(), "day")} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="supplier"
                label="廠商名稱"
                extra={knownCode ? "由型號決定" : "新型號才需要填"}
              >
                {knownCode ? (
                  <Input disabled />
                ) : (
                  <CreatableSelect options={suppliers} placeholder="選廠商" addLabel="新增廠商"
                                   />
                )}
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="item_name"
                label="原物料名稱"
                rules={[{ required: !knownCode, message: "新型號請填原物料名稱" }]}
                extra={knownCode ? "由型號決定" : undefined}
              >
                {knownCode ? (
                  <Input disabled />
                ) : (
                  <CreatableSelect options={materialNames} placeholder="選原物料名稱"
                                   addLabel="新增名稱" />
                )}
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="item_code"
                label="型號"
                rules={[{ required: true, message: "請填型號" }]}
                extra="可直接打新型號，系統會一併建品項"
              >
                <CreatableSelect
                  options={items.map((i) => i.item_code)}
                  describe={(code) => {
                    const item = byCode.get(code);
                    if (!item) return code;
                    return `${code}｜${item.name}${item.meters_per_box ? `（每箱 ${item.meters_per_box} 米）` : ""}`;
                  }}
                  onChange={onCodeChange}
                  placeholder="選型號"
                  addLabel="新增型號"
                />
              </Form.Item>
            </Col>
          </Row>

          <Row gutter={24}>
            <Col xs={24} md={6}>
              <Form.Item
                label={
                  <span>
                    數量{" "}
                    <Segmented
                      size="small"
                      value={qtyMode}
                      onChange={(v) => setQtyMode(v as "米" | "箱")}
                      options={["米", "箱"]}
                      style={{ marginLeft: 8 }}
                    />
                  </span>
                }
                required
                extra={
                  qtyMode === "米"
                    ? preview
                      ? `= ${preview.boxes} 箱${preview.remainder ? `，剩 ${preview.remainder.toLocaleString()} 米對不上整箱` : ""}`
                      : rate
                        ? `每箱 ${rate.toLocaleString()} 米，輸入米數自動換算`
                        : chosenCode
                          ? `型號 ${chosenCode} 尚未設定每箱米數`
                          : "先填型號"
                    : undefined
                }
                style={{ marginBottom: 0 }}
              >
                {qtyMode === "箱" ? (
                  <Form.Item name="qty" rules={[{ required: true, message: "請填數量" }]} noStyle>
                    <InputNumber min={1} style={{ width: "100%" }} />
                  </Form.Item>
                ) : (
                  <Form.Item name="qty_meters" rules={[{ required: true, message: "請填米數" }]} noStyle>
                    <InputNumber min={1} step={100} style={{ width: "100%" }} />
                  </Form.Item>
                )}
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="spec" label="規格" extra={knownCode ? "由型號決定" : undefined}>
                {knownCode ? (
                  <Input disabled />
                ) : (
                  <CreatableSelect options={specs} placeholder="選規格" addLabel="新增規格"
                                   />
                )}
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="manufacture_date" label="標示（製造日期）" extra="一次到貨含多個製造日 → 分批登錄，一個製造日一批">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="expiry_date"
                label="標示（有效日期）"
                extra="多數包材沒標，留空是正常的"
              >
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="supplier_lot_code" label="原廠批號 ROLL#">
                <Input placeholder="例 20250915-3081*61" />
              </Form.Item>
            </Col>
          </Row>

          <Divider titlePlacement="left" plain>
            <Text type="secondary">檢驗項目</Text>
          </Divider>
          <Row gutter={24}>
            <Col xs={24} md={14}>
              <Form.Item name="inspection" label="逐項確認">
                <Checkbox.Group options={INSPECTION as unknown as string[]} />
              </Form.Item>
            </Col>
            <Col xs={24} md={4}>
              <Form.Item name="verdict" label="判定">
                <Radio.Group buttonStyle="solid">
                  <Radio.Button value="合格">合格</Radio.Button>
                  <Radio.Button value="不合格">不合格</Radio.Button>
                </Radio.Group>
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="remark" label="備註">
                <Input placeholder="不合格請寫原因" />
              </Form.Item>
            </Col>
          </Row>
          {verdict === "不合格" && (
            <Alert
              type="warning"
              title="判定不合格：會留紀錄，但不進可領用庫存"
              description="這批仍然存在系統裡（誰收的、什麼時候、為什麼不合格），只是不計入在庫、FIFO 不會指到它、也領不出來。請在備註寫原因。"
              style={{ marginBottom: 16 }}
            />
          )}

          <Row gutter={24}>
            <Col xs={24} md={6}>
              <Form.Item label="記錄人">
                {/* Not a field. Signing as whoever is logged in is the whole
                    point — a dropdown of names lets anyone sign as anyone, and
                    then dual sign-off controls nothing. */}
                <Input value={user?.name ?? ""} disabled suffix={<Text type="secondary">登入身分</Text>} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="confirmed_by"
                label="確認人"
                validateStatus={sameSigner ? "warning" : undefined}
                help={sameSigner ? "跟記錄人是同一個人" : undefined}
              >
                <Select
                  allowClear
                  placeholder="選覆核的人"
                  options={roster
                    .filter((r) => r.name !== user?.name)
                    .map((r) => ({ value: r.name, label: `${r.name}（${r.title ?? r.role_label}）` }))}
                />
              </Form.Item>
            </Col>
          </Row>

          <Divider titlePlacement="left" plain>
            <Text type="secondary">品項設定（影響提醒門檻）</Text>
          </Divider>
          <Row gutter={24}>
            <Col xs={24} md={6}>
              <Form.Item name="shelf_life_days" label="保存期限（天）" extra="留空則不發效期提醒">
                <InputNumber min={1} style={{ width: "100%" }} placeholder="例 540" />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="safety_stock" label="安全水位（箱）" extra="低於此值發低水位提醒">
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="meters_per_box"
                label="每箱米數（米）"
                extra="設了才能用米數收貨；留空代表此品項只用箱計"
              >
                <InputNumber min={1} step={100} style={{ width: "100%" }} placeholder="例 600" />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="supplier_code"
                label="箱上完整料號"
                extra="影像辨識用來對回型號，不填不影響收貨"
              >
                <Input placeholder="例 2003.T7320BC-340X900-P1" />
              </Form.Item>
            </Col>
          </Row>
          {/* Only once a 型號 is on the form. Warning before they have typed
              anything is noise, and noise is how a real warning gets ignored. */}
          {qtyMode === "米" && Boolean(chosenCode) && !rate && (
            <Alert
              type="warning"
              title="要用米數收貨，得先設定每箱米數"
              description="就填在上面「每箱米數」，或到「品項與米數」統一維護。庫存單位仍然是箱 —— 一箱＝一捲，不做部分入庫。"
              style={{ marginBottom: 16 }}
            />
          )}

          <Button type="primary" size="large" icon={<PlusOutlined />} loading={busy} onClick={submit}>
            建立批次
          </Button>
        </Form>
      </Card>

      <Card
        title="在庫批次"
        style={{ marginTop: 24 }}
        extra={
          lots.some((l) => l.qty_on_hand > 0) ? (
            <Link href="/issue">
              <Button type="primary" icon={<ArrowRightOutlined />}>去領用登錄</Button>
            </Link>
          ) : null
        }
      >
        <Table
          rowKey="id"
          dataSource={lots}
          pagination={false}
          size="middle"
          locale={{ emptyText: <Empty description="尚無批次" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          columns={[
            { title: "型號", dataIndex: "item_code" },
            { title: "原物料名稱", dataIndex: "item_name" },
            { title: "廠商", dataIndex: "supplier", render: (v) => v ?? "—" },
            {
              title: "進貨日",
              dataIndex: "receipt_date",
              render: (v: string, row: Lot) => (
                <span>
                  {v} {row.is_fifo_next && row.qty_on_hand > 0 && <Tag color="green">FIFO 應領</Tag>}
                </span>
              ),
            },
            { title: "製造日", dataIndex: "manufacture_date", render: (v) => v ?? "—" },
            { title: "有效日", dataIndex: "expiry_date", render: (v) => v ?? "—" },
            {
              title: "判定",
              dataIndex: "verdict",
              render: (v: string | null) =>
                v === "不合格" ? <Tag color="red">不合格</Tag>
                  : v === "合格" ? <Tag color="green">合格</Tag>
                  : <Text type="secondary">—</Text>,
            },
            { title: "原廠批號", dataIndex: "supplier_lot_code", render: (v) => v ?? "—" },
            {
              title: "在庫",
              dataIndex: "qty_on_hand",
              align: "right" as const,
              render: (v: number, row: Lot) => (
                v > 0 ? (
                  <span>
                    {v} 箱
                    {row.entered_meters ? (
                      <Text type="secondary">（單 {row.entered_meters.toLocaleString()} 米）</Text>
                    ) : null}
                  </span>
                ) : <Text type="secondary">已用完</Text>
              ),
            },
            {
              title: "",
              width: 60,
              render: (_, row: Lot) =>
                row.qty_on_hand > 0 && row.verdict !== "不合格" ? (
                  <Link href={`/issue?item=${encodeURIComponent(row.item_code)}`}>領用</Link>
                ) : null,
            },
            {
              title: "管理",
              width: 130,
              hidden: !can("lot.edit") && !can("lot.delete"),
              render: (_, row: Lot) => (
                <Space size={4}>
                  <Button
                    size="small"
                    type="link"
                    onClick={() => {
                      setEditing(row);
                      editForm.setFieldsValue({
                        receipt_date: dayjs(row.receipt_date),
                        manufacture_date: row.manufacture_date ? dayjs(row.manufacture_date) : null,
                        expiry_date: row.expiry_date ? dayjs(row.expiry_date) : null,
                        supplier: row.supplier,
                        supplier_lot_code: row.supplier_lot_code,
                        qty_on_hand: row.qty_on_hand,
                        verdict: row.verdict,
                        remark: row.remark,
                      });
                    }}
                  >
                    編輯
                  </Button>
                  {row.draw_count > 0 ? (
                    // Not disabled-and-silent: the reason is the interesting part.
                    <Tooltip title={`已有 ${row.draw_count} 筆領用紀錄，刪掉追溯會斷。要退出流通請把數量改成 0。`}>
                      <Button size="small" type="link" disabled>刪除</Button>
                    </Tooltip>
                  ) : (
                    <Popconfirm
                      title="刪除這批？"
                      description="沒有任何領用紀錄，可以安全刪除。"
                      okText="刪除"
                      cancelText="取消"
                      okButtonProps={{ danger: true }}
                      onConfirm={() => removeLot(row)}
                    >
                      <Button size="small" type="link" danger>刪除</Button>
                    </Popconfirm>
                  )}
                </Space>
              ),
            },
          ]}
        />
      </Card>

      <Modal
        open={Boolean(editing)}
        title={`編輯批次 #${editing?.id}　${editing?.item_code ?? ""}`}
        onCancel={() => setEditing(null)}
        onOk={saveEdit}
        confirmLoading={busy}
        okText="儲存"
        cancelText="取消"
        width={800}
        destroyOnHidden
      >
        <Alert
          type="info"
          title="變更會寫入異動軌跡"
          description={
            editing?.draw_count
              ? `這批已有 ${editing.draw_count} 筆領用紀錄。改進貨日會改變 FIFO 順序，改數量會影響既有帳 —— 變更前後值都會留紀錄。`
              : "改動前後的值都會留下紀錄，不會靜默變更。"
          }
          style={{ marginBottom: 16 }}
        />
        <Form form={editForm} layout="vertical">
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="receipt_date" label="進貨日期" extra="FIFO 排序鍵">
                <DatePicker style={{ width: "100%" }} disabledDate={(d) => d.isAfter(dayjs(), "day")} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="manufacture_date" label="標示（製造日期）">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="expiry_date" label="標示（有效日期）">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="supplier" label="廠商名稱">
                <CreatableSelect options={suppliers} placeholder="選廠商" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="supplier_lot_code" label="原廠批號 ROLL#">
                <Input />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="qty_on_hand" label="在庫數量（箱）">
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="verdict" label="判定">
                <Radio.Group buttonStyle="solid">
                  <Radio.Button value="合格">合格</Radio.Button>
                  <Radio.Button value="不合格">不合格</Radio.Button>
                </Radio.Group>
              </Form.Item>
            </Col>
            <Col span={16}>
              <Form.Item name="remark" label="備註">
                <Input />
              </Form.Item>
            </Col>
          </Row>
        </Form>
      </Modal>
    </>
  );
}
