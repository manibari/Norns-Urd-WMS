"use client";

/**
 * 系統設定 — the knobs that apply to the whole plant.
 *
 * Distinct from 基本資料, which is the data the work is *about* (items,
 * suppliers, machines). These are decisions about how the system behaves: which
 * model reads the labels, where images come from, when an alert fires. Mixing
 * the two makes it unclear which changes affect a record and which affect the
 * software.
 */

import { Alert, Button, Card, Col, Form, InputNumber, Radio, Row, Space, Tabs, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import CameraSettingsPanel from "@/components/CameraSettingsPanel";
import { useAuth } from "@/components/AuthGate";
import { api } from "@/lib/api";

const { Title, Text } = Typography;

export default function SettingsPage() {
  const { can } = useAuth();
  const readOnly = !can("dictionary.manage");

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>系統設定</Title>
      <Text type="secondary" style={{ display: "block", marginBottom: 16 }}>
        整廠共用的設定。品項、廠商、機台那些「資料」在「基本資料」，這裡是「系統怎麼運作」。
      </Text>

      <Card>
        <Tabs
          items={[
            { key: "vision", label: "影像設定", children: <VisionSettings readOnly={readOnly} /> },
            { key: "alerts", label: "提醒門檻", children: <AlertSettings readOnly={readOnly} /> },
          ]}
        />
      </Card>
    </>
  );
}

function VisionSettings({ readOnly }: { readOnly: boolean }) {
  const [model, setModel] = useState<string>();
  const [models, setModels] = useState<{ value: string; label: string; note: string }[]>([]);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const s = await api.recognitionSettings();
      setModel(s.model);
      setModels(s.models);
    } catch (e) {
      message.error((e as Error).message);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  async function save(next: string) {
    setBusy(true);
    try {
      await api.saveRecognition(next);
      setModel(next);
      message.success("已更改辨識模型，下一張照片就會用新的");
    } catch (e) {
      message.error((e as Error).message);
      load();
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Title level={5} style={{ marginTop: 0 }}>辨識模型</Title>
      <Text type="secondary" style={{ display: "block", marginBottom: 12 }}>
        讀標籤型號與驗收章的模型。下面的秒數是拿現場實照量出來的中位值，不是估計。
      </Text>
      <Radio.Group
        value={model}
        disabled={readOnly || busy}
        onChange={(e) => save(e.target.value)}
        style={{ display: "block", marginBottom: 32 }}
      >
        <Space orientation="vertical" size={8}>
          {models.map((m) => (
            <Radio key={m.value} value={m.value}>
              <Space>
                <span>{m.label}</span>
                <Text type="secondary" style={{ fontSize: 12 }}>{m.note}</Text>
              </Space>
            </Radio>
          ))}
        </Space>
      </Radio.Group>

      <Title level={5}>影像來源</Title>
      <CameraSettingsPanel />
    </>
  );
}

function AlertSettings({ readOnly }: { readOnly: boolean }) {
  const [form] = Form.useForm();
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      form.setFieldsValue(await api.alertThresholds());
    } catch (e) {
      message.error((e as Error).message);
    }
  }, [form]);

  useEffect(() => { load(); }, [load]);

  async function save() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      await api.saveAlertThresholds(values);
      message.success("已更新提醒門檻");
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Alert
        type="info"
        title="這些數字決定「提醒」頁上什麼時候跳出來"
        description="調寬會漏掉真正該處理的；調窄會天天跳、然後被當背景雜訊忽略 —— 兩種失效方式都存在。"
        style={{ marginBottom: 24 }}
      />
      <Form form={form} layout="vertical" disabled={readOnly} style={{ maxWidth: 640 }}>
        <Row gutter={16}>
          <Col xs={24} md={8}>
            <Form.Item name="expiry_days" label="效期將屆（天）" extra="剩多少天以內開始提醒">
              <InputNumber min={1} max={3650} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
          <Col xs={24} md={8}>
            <Form.Item name="stale_days" label="呆滯批次（天）" extra="進貨後放多久還沒領完">
              <InputNumber min={1} max={3650} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
          <Col xs={24} md={8}>
            <Form.Item name="pending_hours" label="明細待補（小時）" extra="領走後多久還沒補產品名稱">
              <InputNumber min={1} max={720} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
        </Row>
        <Button type="primary" onClick={save} loading={busy} disabled={readOnly}>儲存</Button>
      </Form>
    </>
  );
}
