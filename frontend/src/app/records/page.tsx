"use client";

/**
 * Records and traceability.
 *
 * Blocked draws are listed alongside posted ones, not filed away as errors.
 * That is the product: a blocked record says who, when, which machine, which
 * lot they took, which lot they should have taken, and the photo — permanently
 * (requirement section 2.1). It also has to appear in traceability, because the
 * system can block the books but not the physical box (risk R3).
 */

import { Button, Card, Descriptions, Empty, Form, Input, Modal, Select, Space, Table, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { api, type Dictionary, type Scan } from "@/lib/api";

const { Title, Text } = Typography;

const STATUS: Record<Scan["status"], { color: string; label: string }> = {
  posted: { color: "green", label: "已登錄" },
  overridden: { color: "orange", label: "已覆核放行" },
  blocked_fifo: { color: "red", label: "非 FIFO" },
  blocked_unreadable: { color: "orange", label: "待人工補批次" },
  voided: { color: "default", label: "已作廢" },
};



export default function RecordsPage() {
  const [rows, setRows] = useState<Scan[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const reasons = (dict?.entries.override_reason ?? []).map((e) => e.value);
  const [target, setTarget] = useState<Scan | null>(null);
  const [busy, setBusy] = useState(false);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    try {
      const [scans, d] = await Promise.all([api.scans(), api.dictionary()]);
      setRows(scans);
      setDict(d);
    } catch (e) {
      message.error((e as Error).message);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  async function confirmOverride() {
    const values = await form.validateFields().catch(() => null);
    if (!values || !target) return;
    const reason = values.reason === "其他" ? values.detail : values.reason;
    setBusy(true);
    try {
      await api.override(target.id, reason);
      message.success("已放行並扣庫存");
      setTarget(null);
      form.resetFields();
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const blocked = rows.filter((r) => r.status === "blocked_fifo").length;
  const pending = rows.filter((r) => r.detail_pending && ["posted", "overridden"].includes(r.status)).length;

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>紀錄與追溯</Title>

      <Card title="領用紀錄">
        <Space size={24} style={{ marginBottom: 16 }}>
          <Text>共 {rows.length} 筆</Text>
          {blocked > 0 && <Text type="danger">{blocked} 筆非 FIFO 待覆核</Text>}
          {pending > 0 && <Text type="warning">{pending} 筆包裝產品待補</Text>}
        </Space>
        <Table
          rowKey="id"
          dataSource={rows}
          pagination={false}
          size="middle"
          locale={{ emptyText: <Empty description="尚無領用紀錄" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          expandable={{
            expandedRowRender: (row) => (
              <Descriptions size="small" column={2} bordered>
                <Descriptions.Item label="辨識讀到">{row.ocr_receipt_date ?? "—"}</Descriptions.Item>
                <Descriptions.Item label="辨識信心">
                  {row.ocr_confidence != null ? `${(row.ocr_confidence * 100).toFixed(0)}%` : "—"}
                </Descriptions.Item>
                <Descriptions.Item label="FIFO 應領">{row.fifo_expected_date ?? "—"}</Descriptions.Item>
                <Descriptions.Item label="登記人">{row.captured_by}</Descriptions.Item>
                {Object.entries(row.field_values).map(([k, v]) => (
                  <Descriptions.Item key={k} label={k}>{v || "—"}</Descriptions.Item>
                ))}
                {row.override_reason && (
                  <Descriptions.Item label="覆核原因" span={2}>
                    {row.override_reason}（{row.override_by}）
                  </Descriptions.Item>
                )}
                {row.image_path && (
                  <Descriptions.Item label="影像" span={2}>
                    {/* eslint-disable-next-line @next/next/no-img-element */}
                    <img src={row.image_path} alt="領用影像" style={{ maxWidth: 320, borderRadius: 6 }} />
                  </Descriptions.Item>
                )}
              </Descriptions>
            ),
          }}
          columns={[
            { title: "#", dataIndex: "id", width: 64 },
            { title: "時間", dataIndex: "captured_at", render: (v: string) => v.slice(0, 16).replace("T", " ") },
            { title: "品名", dataIndex: "item_name", render: (v) => v ?? "—" },
            { title: "取用批次", dataIndex: "receipt_date", render: (v) => v ?? "—" },
            {
              title: "狀態",
              dataIndex: "status",
              render: (v: Scan["status"], row) => (
                <Space>
                  <Tag color={STATUS[v].color}>{STATUS[v].label}</Tag>
                  {row.detail_pending === 1 && <Tag color="orange">產品待補</Tag>}
                </Space>
              ),
            },
            { title: "機台", render: (_, row) => row.field_values["包裝機台"] ?? "—" },
            { title: "包裝產品", render: (_, row) => row.field_values["包裝產品"] || <Text type="secondary">待補</Text> },
            {
              title: "",
              width: 100,
              render: (_, row) =>
                row.status === "blocked_fifo" ? (
                  <Button size="small" onClick={() => setTarget(row)}>覆核放行</Button>
                ) : null,
            },
          ]}
        />
      </Card>

      <Modal
        open={Boolean(target)}
        title={`覆核放行 #${target?.id}`}
        onCancel={() => setTarget(null)}
        onOk={confirmOverride}
        confirmLoading={busy}
        okText="放行並扣庫存"
        cancelText="取消"
        width={520}
        destroyOnHidden
      >
        <Text type="secondary">
          放行後這筆會扣庫存並改為「已覆核放行」，原因寫入稽核軌跡。沒有這條路，現場就會學會不拍照直接用。
        </Text>
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item name="reason" label="原因" rules={[{ required: true, message: "必須填原因" }]}>
            <Select options={reasons.map((r) => ({ value: r, label: r }))} placeholder="選擇原因" />
          </Form.Item>
          <Form.Item
            noStyle
            shouldUpdate={(prev, next) => prev.reason !== next.reason}
          >
            {({ getFieldValue }) =>
              getFieldValue("reason") === "其他" ? (
                <Form.Item name="detail" label="說明" rules={[{ required: true, message: "請說明" }]}>
                  <Input.TextArea rows={3} />
                </Form.Item>
              ) : null
            }
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
