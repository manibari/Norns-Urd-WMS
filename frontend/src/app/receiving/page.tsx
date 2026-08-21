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
  Alert, Button, Card, Col, DatePicker, Divider, Form, Input,
  InputNumber, Radio, Row, Segmented, Select, Space, Steps, Typography, message,
} from "antd";
import dayjs from "dayjs";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import CreatableSelect from "@/components/CreatableSelect";
import { api, type Dictionary, type Item } from "@/lib/api";

const { Title, Text } = Typography;

/** A block heading inside the wizard step. */
function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: 15, fontWeight: 600, marginTop: 8, marginBottom: 4,
        paddingBottom: 8, borderBottom: "2px solid #1677ff", display: "inline-block",
      }}
    >
      {children}
    </div>
  );
}

/**
 * 正常 / 不正常, with a note required when it is not.
 *
 * "不正常" without a reason is an inspection that recorded a problem and then
 * lost what it was — the note is the only part a recall can act on, so it is
 * required rather than optional.
 */
function CheckResult({ field }: { field: string }) {
  return (
    <Space orientation="vertical" size={8} style={{ width: "100%" }}>
      <Form.Item name={`${field}_result`} noStyle>
        <Radio.Group buttonStyle="solid">
          <Radio.Button value="正常">正常</Radio.Button>
          <Radio.Button value="不正常">不正常</Radio.Button>
        </Radio.Group>
      </Form.Item>
      <Form.Item
        noStyle
        shouldUpdate={(a, b) => a[`${field}_result`] !== b[`${field}_result`]}
      >
        {({ getFieldValue }) => getFieldValue(`${field}_result`) === "不正常" ? (
          <Form.Item
            name={`${field}_note`} noStyle
            rules={[{ required: true, message: "不正常請說明" }]}
          >
            <Input placeholder="哪裡不正常？" style={{ width: 360 }} />
          </Form.Item>
        ) : null}
      </Form.Item>
    </Space>
  );
}

/** One numbered line of the inspection checklist: number, label, control, hint. */
function InspectionRow({
  n, label, hint, required, children,
}: {
  n?: number;
  label: string;
  hint?: string;
  required?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex", alignItems: "flex-start", gap: 12,
        padding: "10px 0", borderBottom: "1px solid #f0f0f0",
      }}
    >
      {/* Fixed 32px line boxes so the number and label line up with the first
          row of the control, even when the control stacks a note underneath. */}
      <span
        style={{
          width: 22, flexShrink: 0, height: 32, display: "flex",
          alignItems: "center", justifyContent: "flex-end", color: "#8c8c8c",
        }}
      >
        {n ?? ""}
      </span>
      <span
        style={{
          width: 96, flexShrink: 0, height: 32, display: "flex", alignItems: "center",
        }}
      >
        {required && <span style={{ color: "#ff4d4f", marginInlineEnd: 4 }}>*</span>}
        {label}
      </span>
      <div style={{ flex: 1, minWidth: 0 }}>
        {children}
        {hint && (
          <div style={{ marginTop: 6 }}>
            <Text type="secondary" style={{ fontSize: 12 }}>{hint}</Text>
          </div>
        )}
      </div>
    </div>
  );
}

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
          規格尺寸: values.spec_result ?? null,
          規格尺寸備註: values.spec_note || null,
          標示製造日期: values.manufacture_date ? "已填" : null,
          標示有效日期: values.expiry_date ? "已填" : null,
          外觀: values.appearance_result ?? null,
          外觀備註: values.appearance_note || null,
          顏色: values.colour_result ?? null,
          顏色備註: values.colour_note || null,
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
        "spec_result", "spec_note", "appearance_result", "appearance_note",
        "colour_result", "colour_note", "verdict", "remark",
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
              form.setFieldsValue({
                receipt_date: dayjs(), verdict: "合格",
                spec_result: "相符", appearance_result: "正常", colour_result: "正常",
              });
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
          initialValues={{
            qty: 1, receipt_date: dayjs(), verdict: "合格",
            spec_result: "相符", appearance_result: "正常", colour_result: "正常",
          }}
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
              description="對應驗收單的「檢驗項目」欄，一項一項往下確認。"
              style={{ marginBottom: 24 }}
            />

            {/* One row per item, numbered, top to bottom. Laid out across three
                columns it read as a wall of controls; the paper form is a
                checklist and so is this. */}
            <div style={{ maxWidth: 760 }}>
              <SectionHeading>檢驗項目</SectionHeading>
              {/* Two explicit options rather than a checkbox: an unticked box
                  cannot say whether the item failed or was never looked at, and
                  those are very different things on an inspection record. */}
              <InspectionRow n={1} label="規格尺寸" hint="核對實物與主檔登記的規格">
                <Space orientation="vertical" size={8} style={{ width: "100%" }}>
                  <Space wrap>
                    <Input
                      value={form.getFieldValue("spec") || picked?.spec || "（未登記規格）"}
                      disabled
                      style={{ width: 200 }}
                    />
                    <Form.Item name="spec_result" noStyle>
                      <Radio.Group buttonStyle="solid">
                        <Radio.Button value="相符">相符</Radio.Button>
                        <Radio.Button value="不符">不符</Radio.Button>
                      </Radio.Group>
                    </Form.Item>
                  </Space>
                  <Form.Item noStyle shouldUpdate={(a, b) => a.spec_result !== b.spec_result}>
                    {({ getFieldValue }) => getFieldValue("spec_result") === "不符" ? (
                      <Form.Item
                        name="spec_note" noStyle
                        rules={[{ required: true, message: "不符請說明" }]}
                      >
                        <Input placeholder="哪裡不符？" style={{ width: 360 }} />
                      </Form.Item>
                    ) : null}
                  </Form.Item>
                </Space>
              </InspectionRow>

              <InspectionRow n={2} label="製造日期" hint="同進貨日時，用來決定哪一批先領">
                <Form.Item name="manufacture_date" noStyle>
                  <DatePicker style={{ width: 220 }} placeholder="選日期" />
                </Form.Item>
              </InspectionRow>

              <InspectionRow
                n={3}
                label="有效期限"
                required={needsExpiry}
                hint={needsExpiry ? "這個品項有效期，必須填" : "此品項未標記有效期，留空是正常的"}
              >
                <Form.Item
                  name="expiry_date"
                  noStyle
                  rules={[{ required: needsExpiry, message: "此品項有效期，必須填有效期限" }]}
                >
                  <DatePicker style={{ width: 220 }} placeholder="選日期" />
                </Form.Item>
              </InspectionRow>

              <InspectionRow n={4} label="外觀">
                <CheckResult field="appearance" />
              </InspectionRow>

              <InspectionRow n={5} label="顏色">
                <CheckResult field="colour" />
              </InspectionRow>

              <InspectionRow n={6} label="判定" hint="不合格會留紀錄，但不進可領用庫存">
                <Form.Item name="verdict" noStyle>
                  <Radio.Group buttonStyle="solid">
                    <Radio.Button value="合格">合格</Radio.Button>
                    <Radio.Button value="不合格">不合格</Radio.Button>
                  </Radio.Group>
                </Form.Item>
              </InspectionRow>

              <InspectionRow label="備註">
                <Form.Item name="remark" noStyle>
                  <Input placeholder="不合格請寫原因" style={{ width: 360 }} />
                </Form.Item>
              </InspectionRow>

              <Form.Item
                noStyle
                shouldUpdate={(a, b) =>
                  a.verdict !== b.verdict || a.appearance_result !== b.appearance_result
                  || a.colour_result !== b.colour_result || a.spec_result !== b.spec_result}
              >
                {({ getFieldValue }) => {
                  const failed = [
                    getFieldValue("spec_result") === "不符" && "規格尺寸",
                    getFieldValue("appearance_result") === "不正常" && "外觀",
                    getFieldValue("colour_result") === "不正常" && "顏色",
                  ].filter(Boolean);
                  if (!failed.length || getFieldValue("verdict") === "不合格") return null;
                  return (
                    <Alert
                      type="warning"
                      title={`${failed.join("、")}有異常，判定卻是合格`}
                      description="這樣寫是可以的（有些瑕疵不影響使用），但請確認是刻意的 —— 異常內容會留在紀錄裡。"
                      style={{ marginBlock: 16 }}
                    />
                  );
                }}
              </Form.Item>

              <Form.Item noStyle shouldUpdate={(a, b) => a.verdict !== b.verdict}>
                {({ getFieldValue }) => getFieldValue("verdict") === "不合格" ? (
                  <Alert
                    type="warning"
                    title="判定不合格：會留紀錄，但不進可領用庫存"
                    description="這批仍然存在系統裡（誰收的、什麼時候、為什麼不合格），只是不計入在庫、FIFO 不會指到、也領不出來。請在備註寫原因。"
                    style={{ marginBlock: 16 }}
                  />
                ) : null}
              </Form.Item>

              <div style={{ marginTop: 28 }}>
                <SectionHeading>簽核</SectionHeading>
              </div>

              <InspectionRow label="記錄人" hint="登入身分，不能改">
                <Input value={user?.name ?? ""} disabled style={{ width: 220 }} />
              </InspectionRow>

              <InspectionRow
                label="確認人"
                hint={sameSigner ? "跟記錄人是同一個人 —— 雙簽的意義就是兩個人" : "覆核這批的人"}
              >
                <Form.Item name="confirmed_by" noStyle>
                  <Select
                    allowClear
                    placeholder="選覆核的人"
                    style={{ width: 220 }}
                    status={sameSigner ? "warning" : undefined}
                    options={roster
                      .filter((r) => r.name !== user?.name)
                      .map((r) => ({ value: r.name, label: `${r.name}（${r.title ?? r.role_label}）` }))}
                  />
                </Form.Item>
              </InspectionRow>
            </div>

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
