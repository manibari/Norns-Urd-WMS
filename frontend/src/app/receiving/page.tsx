"use client";

/**
 * Receiving — where a lot first exists.
 *
 * This screen is the whole reason Urd-WMS can run without an ERP: the FIFO sort
 * key (receipt_date) is entered here, by the person holding the box, at the
 * moment they stamp it. Nothing downstream can be more accurate than this.
 *
 * Every field is free-form, including the item code: receiving is also when a
 * new film first shows up at the factory, and sending the warehouse to a
 * separate master-data screen first is how paper survives.
 */

import { PlusOutlined } from "@ant-design/icons";
import {
  AutoComplete, Button, Card, Col, DatePicker, Divider, Empty, Form, Input,
  InputNumber, Row, Table, Tag, Typography, message,
} from "antd";
import dayjs from "dayjs";
import { useCallback, useEffect, useState } from "react";
import { api, type Item, type Lot } from "@/lib/api";

const { Title, Text } = Typography;

export default function ReceivingPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [lots, setLots] = useState<Lot[]>([]);
  const [busy, setBusy] = useState(false);
  const [knownCode, setKnownCode] = useState(false);
  const [form] = Form.useForm();

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

  function onCodeChange(value: string) {
    const found = items.find((i) => i.item_code === value.trim());
    setKnownCode(Boolean(found));
    if (found) {
      form.setFieldsValue({
        item_name: found.name,
        spec: found.spec,
        shelf_life_days: found.shelf_life_days,
        safety_stock: found.safety_stock,
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
        supplier_lot_code: values.supplier_lot_code || null,
        qty: values.qty,
      });
      message.success(
        res.created_item
          ? `已新增品項並建立批次（進貨日 ${res.receipt_date}）`
          : `已建立批次（進貨日 ${res.receipt_date}）`,
      );
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
            <Col xs={24} md={8}>
              <Form.Item
                name="item_code"
                label="料號"
                rules={[{ required: true, message: "請填料號" }]}
                extra="可直接打新料號，系統會一併建品項"
              >
                <AutoComplete
                  options={items.map((i) => ({ value: i.item_code, label: `${i.item_code}｜${i.name}` }))}
                  filterOption={(input, option) =>
                    String(option?.value ?? "").toLowerCase().includes(input.toLowerCase())
                  }
                  onChange={onCodeChange}
                  placeholder="例 2003.T7320BC-340X900-P1"
                />
              </Form.Item>
            </Col>
            <Col xs={24} md={8}>
              <Form.Item
                name="item_name"
                label="品名"
                rules={[{ required: !knownCode, message: "新料號請填品名" }]}
              >
                <Input placeholder="例 高阻氧食品包裝拉伸膜" />
              </Form.Item>
            </Col>
            <Col xs={24} md={8}>
              <Form.Item name="spec" label="規格">
                <Input placeholder="例 340mm x 900M" />
              </Form.Item>
            </Col>
          </Row>

          <Row gutter={24}>
            <Col xs={24} md={6}>
              <Form.Item
                name="receipt_date"
                label="進貨日（驗收章）"
                rules={[{ required: true, message: "請填進貨日" }]}
                extra="FIFO 就是照這個排序"
              >
                <DatePicker style={{ width: "100%" }} disabledDate={(d) => d.isAfter(dayjs(), "day")} />
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
            <Col xs={24} md={6}>
              <Form.Item name="qty" label="數量（箱）" rules={[{ required: true }]}>
                <InputNumber min={1} style={{ width: "100%" }} />
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
          </Row>

          <Button type="primary" size="large" icon={<PlusOutlined />} loading={busy} onClick={submit}>
            建立批次
          </Button>
        </Form>
      </Card>

      <Card title="在庫批次" style={{ marginTop: 24 }}>
        <Table
          rowKey="id"
          dataSource={lots}
          pagination={false}
          size="middle"
          locale={{ emptyText: <Empty description="尚無批次" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          columns={[
            { title: "料號", dataIndex: "item_code" },
            { title: "品名", dataIndex: "item_name" },
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
              render: (v: number) => (v > 0 ? `${v} 箱` : <Text type="secondary">已用完</Text>),
            },
          ]}
        />
      </Card>
    </>
  );
}
