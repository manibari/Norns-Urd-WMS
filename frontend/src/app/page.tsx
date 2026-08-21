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
import { Button, Card, Col, Empty, Row, Space, Statistic, Table, Tag, Tooltip, Typography, message } from "antd";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import { useColumnWidths } from "@/components/resizable";
import { api, type Alerts, type Item, type Lot } from "@/lib/api";

const { Title, Text } = Typography;

export default function StockPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [lots, setLots] = useState<Lot[]>([]);
  const [alerts, setAlerts] = useState<Alerts | null>(null);
  const { can } = useAuth();
  const cols = useColumnWidths("stock");

  const load = useCallback(async () => {
    const [i, l, a] = await Promise.allSettled([api.items(), api.lots(), api.alerts()]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`品項載入失敗：${i.reason?.message}`);
    if (l.status === "fulfilled") setLots(l.value);
    else message.error(`批次載入失敗：${l.reason?.message}`);
    if (a.status === "fulfilled") setAlerts(a.value);
  }, []);

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
        title="各品項庫存"
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
          scroll={{ x: 1000 }}
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
                  columns={[
                    {
                      title: "進貨日", dataIndex: "receipt_date",
                      render: (v: string, row: Lot) => (
                        <Space>
                          {v}
                          {row.is_fifo_next && row.qty_on_hand > 0 && row.verdict !== "不合格" && (
                            <Tag color="green">FIFO 應領</Tag>
                          )}
                        </Space>
                      ),
                    },
                    { title: "製造日", dataIndex: "manufacture_date", render: (v) => v ?? "—" },
                    { title: "有效日", dataIndex: "expiry_date", render: (v) => v ?? "—" },
                    { title: "廠商", dataIndex: "supplier", render: (v) => v ?? "—" },
                    {
                      title: "判定", dataIndex: "verdict",
                      render: (v: string | null) =>
                        v === "不合格" ? <Tag color="red">不合格</Tag>
                          : v === "合格" ? <Tag color="green">合格</Tag>
                          : <Text type="secondary">—</Text>,
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
    </>
  );
}
