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
  Alert, AutoComplete, Button, Card, Col, DatePicker, Divider, Empty, Form, Input,
  InputNumber, Row, Segmented, Table, Tag, Typography, message,
} from "antd";
import dayjs from "dayjs";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, type Item, type Lot } from "@/lib/api";

const { Title, Text } = Typography;

export default function ReceivingPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [lots, setLots] = useState<Lot[]>([]);
  const [busy, setBusy] = useState(false);
  const [knownCode, setKnownCode] = useState(false);
  // Metres is the default: the acceptance form records quantity in metres, so
  // boxes is the exception here, not the norm.
  const [qtyMode, setQtyMode] = useState<"米" | "箱">("米");
  const [form] = Form.useForm();
  const rate = Form.useWatch("meters_per_box", form) as number | undefined;
  const metres = Form.useWatch("qty_meters", form) as number | undefined;
  const chosenCode = (Form.useWatch("item_code", form) as string | undefined)?.trim();

  // Preview only — the server does the authoritative conversion. Shown live so
  // the remainder is visible before submitting, not discovered afterwards.
  const preview =
    qtyMode === "米" && rate && rate > 0 && metres && metres > 0
      ? { boxes: Math.floor(metres / rate), remainder: metres % rate }
      : null;

  const load = useCallback(async () => {
    try {
      const [i, l] = await Promise.all([api.items(), api.lots()]);
      setItems(i);
      setLots(l);
    } catch (e) {
      message.error((e as Error).message);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  // Suggestions rather than a supplier master: one factory, a handful of names.
  // A second real factory is what would justify the extra table.
  const suppliers = Array.from(
    new Set([...items.map((i) => i.supplier), ...lots.map((l) => l.supplier)].filter(Boolean) as string[]),
  );

  function onCodeChange(value: string) {
    const found = items.find((i) => i.item_code === value.trim());
    setKnownCode(Boolean(found));
    if (found) {
      form.setFieldsValue({
        item_name: found.name,
        spec: found.spec,
        shelf_life_days: found.shelf_life_days,
        safety_stock: found.safety_stock,
        meters_per_box: found.meters_per_box,
        supplier_code: found.supplier_code,
        supplier: found.supplier ?? form.getFieldValue("supplier"),
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
        supplier_lot_code: values.supplier_lot_code || null,
        supplier_code: values.supplier_code || null,
        meters_per_box: values.meters_per_box ?? null,
        ...(qtyMode === "米" ? { qty_meters: values.qty_meters } : { qty: values.qty }),
      });
      message.success(
        `${res.created_item ? "已新增品項並建立批次" : "已建立批次"}：${res.qty} 箱（進貨日 ${res.receipt_date}）`,
      );
      if (res.conversion_note) message.warning(res.conversion_note, 8);
      if (res.same_day_lot_exists) {
        message.warning("同料號同進貨日已有另一批，這次另開一批。要合併請自行調整。", 6);
      }
      form.resetFields();
      setKnownCode(false);
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>收貨建批</Title>

      <Card title="登錄一批到貨">
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
                extra="FIFO 就是照這個排序"
              >
                <DatePicker style={{ width: "100%" }} disabledDate={(d) => d.isAfter(dayjs(), "day")} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="supplier" label="廠商名稱">
                <AutoComplete
                  options={suppliers.map((v) => ({ value: v }))}
                  placeholder="例 臺灣希悅爾"
                />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="item_name"
                label="原物料名稱"
                rules={[{ required: !knownCode, message: "新型號請填原物料名稱" }]}
              >
                <Input placeholder="例 高阻氧食品包裝拉伸膜" />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item
                name="item_code"
                label="型號"
                rules={[{ required: true, message: "請填型號" }]}
                extra="可直接打新型號，系統會一併建品項"
              >
                <AutoComplete
                  options={items.map((i) => ({
                    value: i.item_code,
                    label: `${i.item_code}｜${i.name}${i.meters_per_box ? `（每箱 ${i.meters_per_box} 米）` : ""}`,
                  }))}
                  filterOption={(input, option) =>
                    String(option?.value ?? "").toLowerCase().includes(input.toLowerCase())
                  }
                  onChange={onCodeChange}
                  placeholder="例 T6050BSW"
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
                    <InputNumber min={1} style={{ width: "100%" }} addonAfter="箱" />
                  </Form.Item>
                ) : (
                  <Form.Item name="qty_meters" rules={[{ required: true, message: "請填米數" }]} noStyle>
                    <InputNumber min={1} step={100} style={{ width: "100%" }} addonAfter="米" />
                  </Form.Item>
                )}
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="spec" label="規格">
                <Input placeholder="例 340mm x 900M" />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="manufacture_date" label="製造日（標籤）">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col xs={24} md={6}>
              <Form.Item name="supplier_lot_code" label="原廠批號 ROLL#">
                <Input placeholder="例 20250915-3081*61" />
              </Form.Item>
            </Col>
          </Row>

          <Divider orientation="left" plain>
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
                label="每箱米數"
                extra="設了才能用米數收貨；留空代表此品項只用箱計"
              >
                <InputNumber min={1} step={100} style={{ width: "100%" }} addonAfter="米" placeholder="例 600" />
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
              width: 90,
              render: (_, row: Lot) =>
                row.qty_on_hand > 0 ? (
                  <Link href={`/issue?item=${encodeURIComponent(row.item_code)}`}>領用</Link>
                ) : null,
            },
          ]}
        />
      </Card>
    </>
  );
}
