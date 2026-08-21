"use client";

/**
 * 收貨建批 — two steps, mirroring how the paper 包材驗收單 is actually filled in.
 *
 * Step 1 is the delivery: what arrived, from whom, when, how much. Step 2 is the
 * inspection of it. On paper these are one row, but they are two different acts
 * by two different people at two different moments — the warehouse writes the
 * line, someone inspects and signs it off — and collapsing them into one long
 * form made the screen read as twelve equally-important fields when four of them
 * are the delivery and the rest are a verdict on it.
 *
 * 規格尺寸 is shown from the item master rather than typed: the inspection is
 * "does this match what we expect", which requires the expectation to be on
 * screen. That is also why it is a tick, not a text box — only unusual items get
 * an actual dimension written on the form.
 */

import { ArrowRightOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Checkbox, Col, DatePicker, Divider, Form, Input,
  InputNumber, Radio, Row, Segmented, Select, Space, Steps, Typography, message,
} from "antd";
import dayjs from "dayjs";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import CreatableSelect from "@/components/CreatableSelect";
import { api, type Dictionary, type Item } from "@/lib/api";

const { Title, Text } = Typography;

export default function ReceivingPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [options, setOptions] = useState<{ supplier: string[]; material_name: string[]; spec: string[] }>(
    { supplier: [], material_name: [], spec: [] },
  );
  const [roster, setRoster] = useState<{ name: string; title: string | null; role_label: string }[]>([]);
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);
  const [enteredCount, setEnteredCount] = useState(0);
  const [qtyMode, setQtyMode] = useState<"單上數量" | "箱數">("單上數量");
  const [form] = Form.useForm();
  const { user, can } = useAuth();

  const load = useCallback(async () => {
    const [i, d, o] = await Promise.allSettled([
      api.items(), api.dictionary(), api.itemOptions(),
    ]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`品項載入失敗：${i.reason?.message}`);
    if (d.status === "fulfilled") setDict(d.value);
    if (o.status === "fulfilled") setOptions(o.value);
  }, []);

  useEffect(() => { load(); }, [load]);
  useEffect(() => { api.signers().then(setRoster).catch(() => undefined); }, []);

  const itemId = Form.useWatch("item_id", form) as number | undefined;
  const picked = items.find((i) => i.id === itemId);
  const isNewItem = Boolean(itemId === undefined);
  const rate = (Form.useWatch("meters_per_box", form) as number | undefined) ?? picked?.meters_per_box;
  const packUnit = (Form.useWatch("pack_unit", form) as string | undefined) ?? picked?.pack_unit ?? "";
  const entered = Form.useWatch("qty_meters", form) as number | undefined;
  const confirmedBy = (Form.useWatch("confirmed_by", form) as string | undefined)?.trim();
  const sameSigner = Boolean(confirmedBy && confirmedBy === user?.name);
  const needsExpiry = Boolean(picked?.has_expiry);

  const preview = qtyMode === "單上數量" && rate && rate > 0 && entered && entered > 0
    ? { boxes: Math.floor(entered / rate), remainder: entered % rate }
    : null;

  function onItemChange(value: number | undefined) {
    const found = items.find((i) => i.id === value);
    form.setFieldsValue(found
      ? {
          item_name: found.name, model: found.model, spec: found.spec,
          supplier: found.supplier, meters_per_box: found.meters_per_box,
          pack_unit: found.pack_unit, supplier_code: found.supplier_code,
        }
      : {
          item_name: undefined, model: undefined, spec: undefined, supplier: undefined,
          meters_per_box: undefined, pack_unit: undefined, supplier_code: undefined,
        });
  }

  async function next() {
    const ok = await form.validateFields([
      "item_id", "item_name", "receipt_date", ...(qtyMode === "箱數" ? ["qty"] : ["qty_meters"]),
    ]).catch(() => null);
    if (!ok) return;
    setStep(1);
  }

  async function submit() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      const res = await api.createLot({
        item_id: values.item_id ?? null,
        item_name: values.item_name,
        model: values.model || null,
        spec: values.spec || null,
        supplier: values.supplier || null,
        supplier_code: values.supplier_code || null,
        meters_per_box: values.meters_per_box ?? null,
        pack_unit: values.pack_unit || null,
        receipt_date: dayjs(values.receipt_date).format("YYYY-MM-DD"),
        manufacture_date: values.manufacture_date ? dayjs(values.manufacture_date).format("YYYY-MM-DD") : null,
        expiry_date: values.expiry_date ? dayjs(values.expiry_date).format("YYYY-MM-DD") : null,
        entered_unit: qtyMode,
        inspection: {
          規格尺寸: Boolean(values.spec_ok),
          標示製造日期: Boolean(values.manufacture_date),
          標示有效日期: Boolean(values.expiry_date),
          外觀: Boolean(values.appearance_ok),
          顏色: Boolean(values.colour_ok),
        },
        verdict: values.verdict ?? null,
        confirmed_by: values.confirmed_by || null,
        remark: values.remark || null,
        ...(qtyMode === "單上數量" ? { qty_meters: values.qty_meters } : { qty: values.qty }),
      });
      message.success(
        `${res.created_item ? "已新增品項並建立批次" : "已建立批次"}：${res.qty} 箱（進貨日 ${res.receipt_date}）`,
      );
      if (res.conversion_note) message.warning(res.conversion_note, 8);
      if (res.verdict === "不合格") {
        message.warning("判定不合格：已留紀錄，但不計入可領用庫存，也不會被 FIFO 指到。", 8);
      }
      if (res.duplicate_lot_exists) {
        message.warning("同品項、同進貨日、同製造日已有一批 —— 確認不是重複登錄。", 8);
      }
      if (res.same_signer) {
        message.warning("記錄人與確認人是同一個人 —— 雙簽的意義就是兩個人，請確認。", 8);
      }
      setEnteredCount((n) => n + 1);
      // Header fields repeat down the sheet — on paper they are literally 〃.
      form.resetFields([
        "item_id", "item_name", "model", "spec", "qty", "qty_meters",
        "manufacture_date", "expiry_date", "supplier_code", "meters_per_box", "pack_unit",
        "spec_ok", "appearance_ok", "colour_ok", "verdict", "remark",
      ]);
      setStep(0);
      load();
    } catch (e) {
      message.error((e as Error).message, 8);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>收貨建批</Title>

      <Card
        extra={enteredCount > 0 ? (
          <Space>
            <Text type="secondary">本次已登錄 {enteredCount} 批</Text>
            <Button size="small" onClick={() => {
              form.resetFields();
              form.setFieldsValue({ receipt_date: dayjs(), verdict: "合格" });
              setEnteredCount(0);
              setStep(0);
            }}>清空重填</Button>
          </Space>
        ) : null}
      >
        <Steps
          current={step}
          onChange={(v) => { if (v === 0) setStep(0); else next(); }}
          items={[{ title: "進貨資料" }, { title: "檢驗項目" }]}
          style={{ marginBottom: 24 }}
        />

        <Form
          form={form}
          layout="vertical"
          initialValues={{ qty: 1, receipt_date: dayjs(), verdict: "合格" }}
        >
          <div style={{ display: step === 0 ? "block" : "none" }}>
            <Row gutter={24}>
              <Col xs={24} md={8}>
                <Form.Item name="item_id" label="品項" extra="找不到就留空，下面填名稱新增">
                  <Select
                    allowClear
                    showSearch
                    placeholder="選品項"
                    optionFilterProp="label"
                    onChange={onItemChange}
                    options={items.map((i) => ({
                      value: i.id,
                      label: `${i.label}｜${i.name}${i.spec ? ` ${i.spec}` : ""}`,
                    }))}
                  />
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item
                  name="item_name" label="原物料名稱"
                  rules={[{ required: true, message: "原物料名稱必填" }]}
                  extra={picked ? "由品項決定" : "新品項就填這個"}
                >
                  {picked
                    ? <Input disabled />
                    : <CreatableSelect options={options.material_name} placeholder="選或直接輸入"
                                       addLabel="新增名稱" />}
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item name="model" label="型號" extra={picked ? "由品項決定" : "沒有型號可留空"}>
                  <Input disabled={Boolean(picked)} placeholder="例 T6050BSW" allowClear />
                </Form.Item>
              </Col>
            </Row>

            <Row gutter={24}>
              <Col xs={24} md={8}>
                <Form.Item name="supplier" label="進貨廠商">
                  {picked?.supplier
                    ? <Input disabled />
                    : <CreatableSelect options={options.supplier} placeholder="選廠商"
                                       addLabel="新增廠商" category="supplier" onAdded={load} />}
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item
                  name="receipt_date" label="進貨日期"
                  rules={[{ required: true, message: "請填進貨日期" }]}
                  extra={enteredCount > 0 ? "沿用上一批（單上的〃）" : "FIFO 就是照這個排序"}
                >
                  <DatePicker style={{ width: "100%" }} disabledDate={(d) => d.isAfter(dayjs(), "day")} />
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item
                  label={
                    <span>
                      數量{" "}
                      <Segmented
                        size="small"
                        value={qtyMode}
                        onChange={(v) => setQtyMode(v as "單上數量" | "箱數")}
                        options={["單上數量", "箱數"]}
                        style={{ marginLeft: 8 }}
                      />
                    </span>
                  }
                  required
                  extra={
                    qtyMode === "單上數量"
                      ? preview
                        ? `= ${preview.boxes} 箱${preview.remainder ? `，剩 ${preview.remainder.toLocaleString()}${packUnit} 對不上整箱` : ""}`
                        : rate
                          ? `每箱 ${rate.toLocaleString()}${packUnit}，輸入單上數量自動換算`
                          : "此品項未設每箱數量，請改用箱數，或在下方補設定"
                      : undefined
                  }
                  style={{ marginBottom: 0 }}
                >
                  {qtyMode === "箱數" ? (
                    <Form.Item name="qty" rules={[{ required: true, message: "請填箱數" }]} noStyle>
                      <InputNumber min={1} style={{ width: "100%" }} />
                    </Form.Item>
                  ) : (
                    <Form.Item name="qty_meters" rules={[{ required: true, message: "請填數量" }]} noStyle>
                      <InputNumber min={1} step={100} style={{ width: "100%" }} />
                    </Form.Item>
                  )}
                </Form.Item>
              </Col>
            </Row>

            {isNewItem && (
              <>
                <Divider titlePlacement="left" plain>
                  <Text type="secondary">新品項設定（可事後在基本資料調整）</Text>
                </Divider>
                <Row gutter={24}>
                  <Col xs={24} md={6}>
                    <Form.Item name="spec" label="規格">
                      <CreatableSelect options={options.spec} placeholder="選或直接輸入" addLabel="新增規格" />
                    </Form.Item>
                  </Col>
                  <Col xs={24} md={5}>
                    <Form.Item name="meters_per_box" label="每箱數量" extra="單上數量 ÷ 這個 = 箱數">
                      <InputNumber min={1} step={100} style={{ width: "100%" }} placeholder="例 600" />
                    </Form.Item>
                  </Col>
                  <Col xs={24} md={4}>
                    <Form.Item name="pack_unit" label="單位">
                      <CreatableSelect
                        options={(dict?.entries.pack_unit ?? []).filter((e) => e.active).map((e) => e.value)}
                        placeholder="米／張／包" addLabel="新增單位" />
                    </Form.Item>
                  </Col>
                  <Col xs={24} md={9}>
                    <Form.Item name="supplier_code" label="箱上完整料號" extra="影像辨識用來對回品項">
                      <Input placeholder="例 2003.T7320BC-340X900-P1" />
                    </Form.Item>
                  </Col>
                </Row>
              </>
            )}

            <Button type="primary" size="large" onClick={next} icon={<ArrowRightOutlined />}>
              下一步：檢驗
            </Button>
          </div>

          <div style={{ display: step === 1 ? "block" : "none" }}>
            <Alert
              type="info"
              title={picked || form.getFieldValue("item_name")
                ? `檢驗：${form.getFieldValue("model") || form.getFieldValue("item_name")}`
                : "檢驗項目"}
              description="逐項確認這批貨符不符合，對應驗收單的「檢驗項目」欄。"
              style={{ marginBottom: 24 }}
            />

            <Row gutter={24}>
              <Col xs={24} md={8}>
                <Form.Item label="規格尺寸" extra="主檔登記的規格，核對實物是否相符">
                  <Space>
                    <Input
                      value={form.getFieldValue("spec") || picked?.spec || "（未登記規格）"}
                      disabled
                      style={{ width: 180 }}
                    />
                    <Form.Item name="spec_ok" valuePropName="checked" noStyle>
                      <Checkbox>相符</Checkbox>
                    </Form.Item>
                  </Space>
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item name="manufacture_date" label="製造日期">
                  <DatePicker style={{ width: "100%" }} />
                </Form.Item>
              </Col>
              <Col xs={24} md={8}>
                <Form.Item
                  name="expiry_date" label="有效期限"
                  rules={[{ required: needsExpiry, message: "此品項有效期，必須填有效期限" }]}
                  extra={needsExpiry
                    ? "這個品項有效期，必須填"
                    : "此品項未標記有效期，留空是正常的"}
                >
                  <DatePicker style={{ width: "100%" }} />
                </Form.Item>
              </Col>
            </Row>

            <Row gutter={24}>
              <Col xs={24} md={4}>
                <Form.Item name="appearance_ok" valuePropName="checked" label="外觀">
                  <Checkbox>正常</Checkbox>
                </Form.Item>
              </Col>
              <Col xs={24} md={4}>
                <Form.Item name="colour_ok" valuePropName="checked" label="顏色">
                  <Checkbox>正常</Checkbox>
                </Form.Item>
              </Col>
              <Col xs={24} md={6}>
                <Form.Item name="verdict" label="判定">
                  <Radio.Group buttonStyle="solid">
                    <Radio.Button value="合格">合格</Radio.Button>
                    <Radio.Button value="不合格">不合格</Radio.Button>
                  </Radio.Group>
                </Form.Item>
              </Col>
              <Col xs={24} md={10}>
                <Form.Item name="remark" label="備註">
                  <Input placeholder="不合格請寫原因" />
                </Form.Item>
              </Col>
            </Row>

            <Form.Item noStyle shouldUpdate={(a, b) => a.verdict !== b.verdict}>
              {({ getFieldValue }) => getFieldValue("verdict") === "不合格" ? (
                <Alert
                  type="warning"
                  title="判定不合格：會留紀錄，但不進可領用庫存"
                  description="這批仍然存在系統裡（誰收的、什麼時候、為什麼不合格），只是不計入在庫、FIFO 不會指到、也領不出來。請在備註寫原因。"
                  style={{ marginBottom: 16 }}
                />
              ) : null}
            </Form.Item>

            <Row gutter={24}>
              <Col xs={24} md={6}>
                <Form.Item label="記錄人">
                  <Input value={user?.name ?? ""} disabled suffix={<Text type="secondary">登入身分</Text>} />
                </Form.Item>
              </Col>
              <Col xs={24} md={6}>
                <Form.Item
                  name="confirmed_by" label="確認人"
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

            <Space>
              <Button size="large" onClick={() => setStep(0)}>上一步</Button>
              <Button type="primary" size="large" icon={<PlusOutlined />} loading={busy} onClick={submit}>
                建立批次
              </Button>
            </Space>
          </div>
        </Form>
      </Card>

      <Card style={{ marginTop: 24 }}>
        <Space>
          <Text type="secondary">建好的批次在「庫存總覽」看，也在那裡修正或刪除。</Text>
          <Link href="/"><Button type="link">去庫存總覽</Button></Link>
        </Space>
      </Card>

    </>
  );
}
