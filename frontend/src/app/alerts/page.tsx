"use client";

/**
 * Alerts.
 *
 * The panel always renders the check time, even when nothing is wrong. A blank
 * screen cannot distinguish "nothing is wrong" from "the check stopped running"
 * (M7 architecture, constraint 2) — and a monitoring surface that can be
 * silently dead is worse than none, because people trust it.
 */

import { CheckCircleOutlined } from "@ant-design/icons";
import { Card, Col, Empty, Row, Statistic, Table, Tag, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { api, type Alerts } from "@/lib/api";

const { Title, Text } = Typography;

export default function AlertsPage() {
  const [data, setData] = useState<Alerts | null>(null);

  useEffect(() => {
    api.alerts().then(setData).catch((e) => message.error(e.message));
  }, []);

  if (!data) return <Card title="提醒" loading />;

  const empty = (text: string) => ({
    emptyText: <Empty description={text} image={Empty.PRESENTED_IMAGE_SIMPLE} />,
  });

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>提醒</Title>

      <Card>
        <Row gutter={24}>
          <Col xs={12} md={6}><Statistic title="效期將屆" value={data.expiring.length} suffix="批" /></Col>
          <Col xs={12} md={6}><Statistic title="呆滯批次" value={data.stale.length} suffix="批" /></Col>
          <Col xs={12} md={6}><Statistic title="低於安全水位" value={data.low_stock.length} suffix="項" /></Col>
          <Col xs={12} md={6}><Statistic title="產品名稱待補" value={data.pending_detail.length} suffix="筆" /></Col>
          <Col xs={12} md={6}>
            <Statistic
              title="驗收不合格待處理"
              value={data.rejected.length}
              suffix="批"
              valueStyle={data.rejected.length ? { color: "#ff4d4f" } : undefined}
            />
          </Col>
        </Row>
        <Text type="secondary" style={{ display: "block", marginTop: 16 }}>
          <CheckCircleOutlined style={{ color: "#52c41a", marginRight: 6 }} />
          最後檢查 {data.checked_at.slice(0, 19).replace("T", " ")}
          ｜門檻：效期 {data.thresholds.expiry_days} 天內、呆滯 {data.thresholds.stale_days} 天、
          明細待補 {data.thresholds.pending_hours} 小時
          ｜到期日以包材標示為準，沒標示才用製造日＋保存期限推算
        </Text>
      </Card>

      <Card title="驗收不合格待處理" style={{ marginTop: 24 }}>
        <Table
          rowKey="id"
          size="middle"
          pagination={false}
          dataSource={data.rejected}
          locale={empty("沒有不合格批次")}
          columns={[
            { title: "型號", dataIndex: "model", align: "center" as const, render: (v) => v ?? "—" },
            { title: "原物料名稱", dataIndex: "name", align: "center" as const },
            { title: "廠商", dataIndex: "supplier", render: (v) => v ?? "—" },
            { title: "進貨日", dataIndex: "receipt_date", align: "center" as const },
            { title: "數量", dataIndex: "qty_on_hand", align: "right" as const },
            { title: "原因", dataIndex: "remark", render: (v) => v ?? "—" },
            { title: "記錄／確認", render: (_, row) => `${row.recorded_by ?? "—"}／${row.confirmed_by ?? "—"}` },
          ]}
        />
        <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
          這些批次還在架上但不計入在庫、FIFO 不會指到、也領不出來。退貨或報廢後仍會留著紀錄。
        </Text>
      </Card>

      <Card title="效期將屆" style={{ marginTop: 24 }}>
        <Table
          rowKey="id"
          size="middle"
          pagination={false}
          dataSource={data.expiring}
          locale={empty("尚無將到期批次")}
          columns={[
            { title: "原物料名稱", dataIndex: "name", align: "center" as const },
            { title: "進貨日", dataIndex: "receipt_date", align: "center" as const },
            { title: "製造日", dataIndex: "manufacture_date", align: "center" as const },
            {
              title: "到期日",
              dataIndex: "expires_on",
              render: (v: string, row: { expiry_source?: string }) => (
                <span>
                  {v} <Tag color={row.expiry_source === "標示" ? "blue" : "default"}>{row.expiry_source}</Tag>
                </span>
              ),
            },
            {
              title: "剩餘",
              dataIndex: "days_left",
              align: "right" as const,
              render: (v: number) => <Tag color={v <= 0 ? "red" : v <= 30 ? "orange" : "blue"}>{v} 天</Tag>,
            },
            { title: "在庫", dataIndex: "qty_on_hand", align: "right" as const, render: (v) => `${v} 箱` },
          ]}
        />
      </Card>

      <Card title="呆滯批次" style={{ marginTop: 24 }}>
        <Table
          rowKey="id"
          size="middle"
          pagination={false}
          dataSource={data.stale}
          locale={empty("尚無呆滯批次")}
          columns={[
            { title: "原物料名稱", dataIndex: "name", align: "center" as const },
            { title: "進貨日", dataIndex: "receipt_date", align: "center" as const },
            { title: "停留", dataIndex: "age_days", align: "right" as const, render: (v: number) => `${v} 天` },
            { title: "在庫", dataIndex: "qty_on_hand", align: "right" as const, render: (v) => `${v} 箱` },
          ]}
        />
      </Card>

      <Card title="低於安全水位" style={{ marginTop: 24 }}>
        <Table
          rowKey="item_code"
          size="middle"
          pagination={false}
          dataSource={data.low_stock}
          locale={empty("庫存都在水位之上")}
          columns={[
            { title: "型號", dataIndex: "model", align: "center" as const, render: (v) => v ?? "—" },
            { title: "原物料名稱", dataIndex: "name", align: "center" as const },
            { title: "在庫", dataIndex: "on_hand", align: "right" as const, render: (v) => `${v} 箱` },
            { title: "安全水位", dataIndex: "safety_stock", align: "right" as const, render: (v) => `${v} 箱` },
          ]}
        />
      </Card>

      <Card title="產品名稱待補" style={{ marginTop: 24 }}>
        <Table
          rowKey="id"
          size="middle"
          pagination={false}
          dataSource={data.pending_detail}
          locale={empty("沒有待補明細")}
          columns={[
            { title: "紀錄", dataIndex: "id", render: (v: number) => `#${v}` },
            { title: "型號", dataIndex: "model", align: "center" as const, render: (v) => v ?? "—" },
            { title: "領用時間", dataIndex: "captured_at", render: (v: string) => v.slice(0, 16).replace("T", " ") },
          ]}
        />
      </Card>
    </>
  );
}
