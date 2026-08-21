"use client";

/**
 * 庫存總覽 — the landing screen.
 *
 * The question this answers is the one people actually walk up to the system
 * with: what have we got, and which box should be taken next. So the FIFO
 * target is on the main row rather than hidden behind an expand — knowing there
 * are 9 boxes is useless if you still have to dig to find which one is next.
 *
 * Rejected stock is shown separately from available stock, never summed into
 * it: it is physically on the shelf but must not be drawn, and a single number
 * covering both would be the wrong number for every purpose.
 */

import { ArrowRightOutlined, InboxOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Col, DatePicker, Empty, Form, InputNumber, Modal, Popconfirm,
  Input, Radio, Row, Space, Statistic, Table, Tag, Tooltip, Typography, message,
} from "antd";
import dayjs from "dayjs";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import CreatableSelect from "@/components/CreatableSelect";
import { useAuth } from "@/components/AuthGate";
import ExpiryCell from "@/components/ExpiryCell";
import { useColumnWidths } from "@/components/resizable";
import { api, type Alerts, type Item, type Lot } from "@/lib/api";

const { Title, Text } = Typography;

export default function StockPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [lots, setLots] = useState<Lot[]>([]);
  const [alerts, setAlerts] = useState<Alerts | null>(null);
  const { can } = useAuth();
  const cols = useColumnWidths("stock");
  const [editing, setEditing] = useState<Lot | null>(null);
  const [busy, setBusy] = useState(false);
  const [suppliers, setSuppliers] = useState<string[]>([]);
  const [editForm] = Form.useForm();

  const load = useCallback(async () => {
    const [i, l, a] = await Promise.allSettled([api.items(), api.lots(), api.alerts()]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`品項載入失敗：${i.reason?.message}`);
    if (l.status === "fulfilled") setLots(l.value);
    else message.error(`批次載入失敗：${l.reason?.message}`);
    if (a.status === "fulfilled") setAlerts(a.value);
    api.itemOptions().then((o) => setSuppliers(o.supplier)).catch(() => undefined);
  }, []);

  // Lot corrections live here because this is where lots are looked at. Keeping
  // a second copy of the table on the receiving screen meant two places showing
  // the same rows, one of which was always slightly out of date.
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
        qty_on_hand: values.qty_on_hand,
        verdict: values.verdict || undefined,
        remark: values.remark || undefined,
      });
      message.success("已更新，變更前後值已寫入系統日誌");
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

  useEffect(() => { load(); }, [load]);

  const lotsByItem = new Map<number, Lot[]>();
  for (const lot of lots) {
    if (!lotsByItem.has(lot.item_id)) lotsByItem.set(lot.item_id, []);
    lotsByItem.get(lot.item_id)!.push(lot);
  }

  const totalBoxes = items.reduce((sum, i) => sum + i.on_hand, 0);
  const totalRejected = items.reduce((sum, i) => sum + i.rejected_qty, 0);
  const lowStock = alerts?.low_stock.length ?? 0;
  const expiring = alerts?.expiring.length ?? 0;

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>庫存總覽</Title>

      <Card>
        <Row gutter={24}>
          <Col xs={12} md={6}><Statistic title="品項" value={items.length} suffix="項" /></Col>
          <Col xs={12} md={6}><Statistic title="可領用" value={totalBoxes} suffix="箱" /></Col>
          <Col xs={12} md={6}>
            <Statistic
              title="低於安全水位"
              value={lowStock}
              suffix="項"
              valueStyle={lowStock ? { color: "#faad14" } : undefined}
            />
          </Col>
          <Col xs={12} md={6}>
            <Statistic
              title="效期將屆"
              value={expiring}
              suffix="批"
              valueStyle={expiring ? { color: "#faad14" } : undefined}
            />
          </Col>
        </Row>
        {totalRejected > 0 && (
          <Text type="danger" style={{ display: "block", marginTop: 16 }}>
            另有 {totalRejected} 箱驗收不合格的庫存，不計入可領用，也不會被 FIFO 指到。
          </Text>
        )}
      </Card>

      <Card
        title="各品項庫存（最近進貨的在最上面）"
        style={{ marginTop: 24 }}
        extra={
          <Space>
            {cols.hasCustomWidths && (
              <Button size="small" type="link" onClick={cols.reset}>欄寬還原</Button>
            )}
            {can("lot.create") && (
              <Link href="/receiving"><Space><InboxOutlined />去收貨建批</Space></Link>
            )}
          </Space>
        }
      >
        <Table
          rowKey="id"
          dataSource={items}
          pagination={false}
          size="middle"
          scroll={{ x: 1120 }}
          locale={{ emptyText: <Empty description="尚無品項" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          expandable={{
            expandedRowRender: (item) => {
              const rows = lotsByItem.get(item.id) ?? [];
              return (
                <Table
                  rowKey="id"
                  size="small"
                  pagination={false}
                  dataSource={rows}
                  locale={{ emptyText: "尚無批次" }}
                  onRow={(row: Lot) => ({
                    style: row.qty_on_hand === 0 ? { opacity: 0.45, background: "#fafafa" } : undefined,
                  })}
                  columns={[
                    {
                      title: "進貨日", dataIndex: "receipt_date",
                      render: (v: string, row: Lot) => (
                        <Space size={4} wrap>
                          {v}
                          {row.is_fifo_next && <Tag color="green">FIFO 應領</Tag>}
                          {row.fifo_also_ok && (
                            <Tooltip title="跟應領那批同一個進貨日，領這批也合法">
                              <Tag>同進貨日．可領</Tag>
                            </Tooltip>
                          )}
                        </Space>
                      ),
                    },
                    { title: "製造日", dataIndex: "manufacture_date", render: (v) => v ?? "—" },
                    {
                      title: "有效期限", dataIndex: "effective_expiry",
                      render: (_, row: Lot) => (
                        <ExpiryCell date={row.effective_expiry} daysLeft={row.days_left}
                                    required={row.item_has_expiry === 1} />
                      ),
                    },
                    { title: "廠商", dataIndex: "supplier", render: (v) => v ?? "—" },
                    {
                      title: "判定", dataIndex: "verdict",
                      render: (v: string | null) =>
                        v === "不合格" ? <Tag color="red">不合格</Tag>
                          : v === "合格" ? <Tag color="green">合格</Tag>
                          : <Text type="secondary">—</Text>,
                    },
                    {
                      title: "狀態", dataIndex: "lot_state",
                      render: (v: string, row: Lot) =>
                        v === "已領完" ? <Tag>已領完</Tag>
                          : v === "領貨中"
                            ? <Tag color="blue">領貨中 {row.qty_drawn}/{row.qty_received}</Tag>
                            : <Tag color="default">未動用</Tag>,
                    },
                    {
                      title: "在庫", dataIndex: "qty_on_hand", align: "right" as const,
                      render: (v: number, row: Lot) => (
                        <span>
                          {v} 箱
                          {row.entered_meters ? (
                            <Text type="secondary">（單上 {row.entered_meters.toLocaleString()}）</Text>
                          ) : null}
                        </span>
                      ),
                    },
                    {
                      title: "", width: 120, hidden: !can("lot.edit") && !can("lot.delete"),
                      render: (_, row: Lot) => (
                        <Space size={4}>
                          <Button size="small" type="link" onClick={() => {
                            setEditing(row);
                            editForm.setFieldsValue({
                              receipt_date: dayjs(row.receipt_date),
                              manufacture_date: row.manufacture_date ? dayjs(row.manufacture_date) : null,
                              expiry_date: row.expiry_date ? dayjs(row.expiry_date) : null,
                              supplier: row.supplier, qty_on_hand: row.qty_on_hand,
                              verdict: row.verdict, remark: row.remark,
                            });
                          }}>編輯</Button>
                          {row.draw_count > 0 ? (
                            <Tooltip title={`已有 ${row.draw_count} 筆領用紀錄，刪掉追溯會斷。要退出流通請把數量改成 0。`}>
                              <Button size="small" type="link" disabled>刪除</Button>
                            </Tooltip>
                          ) : (
                            <Popconfirm
                              title="刪除這批？" description="沒有任何領用紀錄，可以安全刪除。"
                              okText="刪除" cancelText="取消" okButtonProps={{ danger: true }}
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
              );
            },
          }}
          columns={cols.resizable([
            { title: "原物料名稱", dataIndex: "name", width: 190,
              render: (v: string) => <Text strong>{v}</Text> },
            { title: "型號", dataIndex: "model", width: 130,
              render: (v: string | null) => v ?? <Text type="secondary">（無）</Text> },
            { title: "規格", dataIndex: "spec", width: 140, render: (v) => v ?? "—" },
            {
              title: "可領用", dataIndex: "on_hand", width: 150, align: "right" as const,
              render: (v: number, row: Item) => (
                <span>
                  <Text strong={v > 0} type={v === 0 ? "secondary" : undefined}>{v} 箱</Text>
                  {row.on_hand_m != null && v > 0 && (
                    <Text type="secondary">
                      （{row.on_hand_m.toLocaleString()}{row.pack_unit ?? ""}）
                    </Text>
                  )}
                </span>
              ),
            },
            {
              title: "批次", dataIndex: "open_lots", width: 80, align: "right" as const,
              render: (v: number) => (v > 0 ? `${v} 批` : <Text type="secondary">—</Text>),
            },
            {
              // The point of the whole screen: not just how much, but which box next.
              title: "下一個該領", width: 170,
              render: (_, item: Item) => {
                const drawable = (lotsByItem.get(item.id) ?? [])
                  .filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格");
                if (!drawable.length) return <Text type="secondary">無可領批次</Text>;
                const next = drawable.reduce((a, b) => (a.receipt_date <= b.receipt_date ? a : b));
                return (
                  <Space>
                    <Tag color="green">進貨 {next.receipt_date}</Tag>
                    {drawable.length > 1 && <Text type="secondary">共 {drawable.length} 批</Text>}
                  </Space>
                );
              },
            },
            {
              title: "最近進貨", dataIndex: "last_receipt_date", width: 120,
              render: (v: string | null) => v ?? <Text type="secondary">—</Text>,
            },
            {
              title: "狀態", width: 150,
              render: (_, item: Item) => {
                const tags = [];
                if (item.safety_stock > 0 && item.on_hand < item.safety_stock) {
                  tags.push(<Tag key="low" color="orange">低於水位 {item.safety_stock}</Tag>);
                }
                if (item.rejected_qty > 0) {
                  tags.push(<Tag key="rej" color="red">{item.rejected_qty} 箱不合格</Tag>);
                }
                if (!item.recognisable) {
                  tags.push(
                    <Tooltip key="man" title="沒有型號也沒登記箱上料號，領用時人工選">
                      <Tag>人工選</Tag>
                    </Tooltip>,
                  );
                }
                return tags.length ? <Space wrap size={4}>{tags}</Space> : <Text type="secondary">—</Text>;
              },
            },
            {
              title: "", width: 70, fixed: "right" as const,
              render: (_, item: Item) =>
                item.on_hand > 0 && can("issue.create") ? (
                  <Link href={`/issue?item=${item.id}`}>
                    <Space size={4}>領用<ArrowRightOutlined /></Space>
                  </Link>
                ) : null,
            },
          ])}
        />
      </Card>

      <Modal
        open={Boolean(editing)}
        title={`編輯批次 #${editing?.id}　${editing?.item_label ?? ""}`}
        onCancel={() => setEditing(null)}
        onOk={saveEdit}
        confirmLoading={busy}
        okText="儲存" cancelText="取消" width={800} destroyOnHidden
      >
        <Alert
          type="info"
          title="變更會寫入系統日誌"
          description={editing?.draw_count
            ? `這批已有 ${editing.draw_count} 筆領用紀錄。改進貨日會改變 FIFO 順序，改數量會影響既有帳 —— 變更前後值都會留紀錄。`
            : "改動前後的值都會留下紀錄，不會靜默變更。"}
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
              <Form.Item name="manufacture_date" label="製造日期" extra="同進貨日時用來決定誰先領">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="expiry_date" label="有效期限">
                <DatePicker style={{ width: "100%" }} />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="supplier" label="進貨廠商">
                <CreatableSelect options={suppliers} placeholder="選廠商" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="qty_on_hand" label="在庫數量（箱）">
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="verdict" label="判定">
                <Radio.Group buttonStyle="solid">
                  <Radio.Button value="合格">合格</Radio.Button>
                  <Radio.Button value="不合格">不合格</Radio.Button>
                </Radio.Group>
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="remark" label="備註">
            <Input />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
