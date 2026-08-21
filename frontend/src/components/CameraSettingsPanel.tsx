"use client";

/**
 * Network camera settings.
 *
 * A fixed camera is an alternative source of the same bytes — everything after
 * capture (recognition, lot matching, FIFO) is untouched. That is why this is a
 * settings panel rather than a second issuing flow.
 *
 * The test button matters more than it looks: a camera that is wired but
 * answering with a login page or an MJPEG stream fails in a way nobody can
 * diagnose from the issuing screen at 7am, so it gets diagnosed here instead.
 */

import { Alert, Button, Col, Form, Input, InputNumber, Row, Select, Space, Switch, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { api, type CameraSettings } from "@/lib/api";

const { Text } = Typography;

function describeAge(seconds: number): string {
  if (seconds < 90) return `${Math.round(seconds)} 秒前`;
  if (seconds < 5400) return `${Math.round(seconds / 60)} 分鐘前`;
  return `${Math.round(seconds / 3600)} 小時前`;
}

export default function CameraSettingsPanel() {
  const [settings, setSettings] = useState<CameraSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    try {
      const s = await api.camera();
      setSettings(s);
      form.setFieldsValue({ ...s, password: "" });
    } catch (e) {
      message.error((e as Error).message);
    }
  }, [form]);

  useEffect(() => { load(); }, [load]);

  const transport = Form.useWatch("transport", form) ?? settings?.transport ?? "http";
  const enabled = Form.useWatch("enabled", form) ?? settings?.enabled ?? false;

  async function save() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      const res = await api.saveCamera(values);
      message.success(`已儲存${res.endpoint ? `：${res.endpoint}` : ""}`);
      setResult(null);
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function test() {
    setBusy(true);
    setResult(null);
    try {
      const res = await api.testCamera();
      // 資料夾來源說「連線成功」沒有意義 —— 要說的是讀到哪個檔、那張多舊。
      // 相機兩小時前就不拍了，資料夾照樣「讀得到」。
      const source = res.source_name
        ? `讀到 ${res.source_name}（${describeAge(res.source_age_seconds ?? 0)}），`
        : "";
      setResult(res.ok
        ? { ok: true, text: `${source}${Math.round((res.bytes ?? 0) / 1024)}KB，耗時 ${res.elapsed_ms}ms` }
        : { ok: false, text: res.error ?? "連線失敗" });
    } catch (e) {
      setResult({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  }

  if (!settings) return null;

  return (
    <>
      <Alert
        type="info"
        title="接一台固定相機當影像來源"
        description="接上之後，領用登錄的「影像辨識」多一個「從相機擷取」—— 拍完直接辨識，跟手機拍照走完全一樣的流程。不接也沒關係，手機拍照本來就能用。"
        style={{ marginBottom: 24 }}
      />

      <Form form={form} layout="vertical" style={{ maxWidth: 720 }}>
        <Form.Item name="enabled" label="啟用網路相機" valuePropName="checked">
          <Switch />
        </Form.Item>

        <Row gutter={16}>
          <Col xs={24} md={transport === "folder" ? 8 : 8}>
            <Form.Item
              name="transport" label="連線方式"
              extra={
                transport === "http" ? "抓一個 snapshot 網址（多數 IP 相機）"
                  : transport === "raw" ? "直接開 TCP socket 讀影像"
                    : "取資料夾裡最新的一張圖"
              }
            >
              <Select
                options={[
                  { value: "http", label: "HTTP snapshot" },
                  { value: "raw", label: "原始 TCP" },
                  { value: "folder", label: "存檔資料夾" },
                ]}
              />
            </Form.Item>
          </Col>
          {transport === "folder" ? (
            <Col xs={24} md={16}>
              <Form.Item
                name="folder" label="影像資料夾"
                rules={[{ required: enabled, message: "啟用時必須填資料夾路徑" }]}
                extra="相機軟體存圖的地方。Windows 磁碟在這裡是 /mnt/c/…"
              >
                <Input placeholder="例 /mnt/c/Users/HP/SCMVS/0821/MV-SC3016C-06M-WBN (DA9442483)" />
              </Form.Item>
            </Col>
          ) : (
            <>
              <Col xs={24} md={10}>
                <Form.Item
                  name="host" label="IP 位址"
                  rules={[{ required: enabled, message: "啟用時必須填位址" }]}
                >
                  <Input placeholder="例 192.168.1.50" />
                </Form.Item>
              </Col>
              <Col xs={24} md={6}>
                <Form.Item name="port" label="連接埠">
                  <InputNumber min={1} max={65535} style={{ width: "100%" }} />
                </Form.Item>
              </Col>
            </>
          )}
        </Row>

        {transport === "http" ? (
          <Row gutter={16}>
            <Col xs={24} md={10}>
              <Form.Item name="path" label="影像路徑" extra="相機的 snapshot 路徑">
                <Input placeholder="/snapshot.jpg" />
              </Form.Item>
            </Col>
            <Col xs={24} md={7}>
              <Form.Item name="username" label="帳號" extra="沒有就留空">
                <Input autoComplete="off" />
              </Form.Item>
            </Col>
            <Col xs={24} md={7}>
              <Form.Item
                name="password" label="密碼"
                extra={settings.has_password ? "已設定，留空不變更" : "沒有就留空"}
              >
                <Input.Password autoComplete="new-password" placeholder={settings.has_password ? "••••••" : ""} />
              </Form.Item>
            </Col>
          </Row>
        ) : (
          <Row gutter={16}>
            <Col xs={24} md={14}>
              <Form.Item
                name="trigger" label="觸發字串"
                extra="連上後先送出的指令，沒有就留空"
              >
                <Input placeholder="例 SNAP\\r\\n" />
              </Form.Item>
            </Col>
          </Row>
        )}

        <Row gutter={16}>
          <Col xs={24} md={8}>
            <Form.Item name="timeout" label="逾時（秒）">
              <InputNumber min={1} max={60} step={1} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
        </Row>

        <Space>
          <Button type="primary" onClick={save} loading={busy}>儲存</Button>
          <Button
            onClick={test}
            loading={busy}
            disabled={settings.transport === "folder" ? !settings.folder : !settings.host}
          >
            測試連線
          </Button>
          {settings.endpoint && <Text type="secondary">目前：{settings.endpoint}</Text>}
        </Space>
      </Form>

      {result && (
        <Alert
          style={{ marginTop: 16, maxWidth: 720 }}
          type={result.ok ? "success" : "error"}
          title={result.ok ? "相機正常" : "連不上相機"}
          description={result.text}
        />
      )}
    </>
  );
}
