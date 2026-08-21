"use client";

/**
 * 人員與權限。
 *
 * 放在「系統設定」而不是「基本資料」：基本資料是流程用得到的資料 —— 沒有型號就
 * 沒東西可收，沒有機台就記不了領用。誰能登入、誰能覆核不是流程的一環，是這套
 * 系統怎麼運作的決定，跟辨識模型、提醒門檻同一類。
 *
 * 權限層級是伺服器在把關的三層，這裡只能改「顯示名稱」和誰屬於哪一層 ——
 * 層級能做什麼是系統設計，不是設定項。
 */

import { PlusOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Form, Input, Modal, Popconfirm,
  Row, Select, Space, Switch, Table, Tag, Tooltip, Typography, message,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import CreatableSelect from "@/components/CreatableSelect";
import { api, type SessionUser } from "@/lib/api";

const { Text } = Typography;

type ManagedUser = SessionUser & { active: number; created_at: string };

/** 自己載自己的資料 —— 它現在跟品項主檔沒有關係，不該再靠那個頁面餵。 */
export default function UsersPanelContainer() {
  const { user: me } = useAuth();
  const [users, setUsers] = useState<ManagedUser[]>([]);
  const [roles, setRoles] = useState<{ code: string; label: string; default_label: string }[]>([]);
  const [jobTitles, setJobTitles] = useState<string[]>([]);

  const load = useCallback(async () => {
    const [u, r, d] = await Promise.allSettled([api.users(), api.roles(), api.dictionary(true)]);
    if (u.status === "fulfilled") setUsers(u.value);
    else message.error(`人員載入失敗：${u.reason?.message}`);
    if (r.status === "fulfilled") setRoles(r.value);
    // 職位沿用字典裡的 job_title，維持跟收貨表單同一份選項
    if (d.status === "fulfilled") {
      setJobTitles((d.value.entries.job_title ?? []).filter((e) => e.active).map((e) => e.value));
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  return (
    <UsersPanel users={users} me={me} roles={roles} jobTitles={jobTitles} onChanged={load} />
  );
}

function UsersPanel({
  users, me, roles, jobTitles, onChanged,
}: {
  users: ManagedUser[];
  me: SessionUser | null;
  roles: { code: string; label: string; default_label: string }[];
  jobTitles: string[];
  onChanged: () => void;
}) {
  const [adding, setAdding] = useState(false);
  const [resetting, setResetting] = useState<ManagedUser | null>(null);
  const [form] = Form.useForm();
  const [pinForm] = Form.useForm();

  // What a role may do is system design and not editable here; what it is
  // called is the factory's vocabulary — 倉管 vs 資材 vs 物管 is a naming
  // difference, not a different set of permissions.
  const SCOPE: Record<string, string> = {
    user: "領用登錄、補明細",
    manager: "加上收貨建批、維護型號、覆核放行、看日誌",
    admin: "加上批次修正與刪除、選項、人員與角色",
  };
  const ROLES = roles.map((r) => ({ value: r.code, label: `${r.label} — ${SCOPE[r.code] ?? ""}` }));

  async function renameRole(code: string, label: string) {
    try {
      await api.patchRole(code, label);
      message.success("角色名稱已更改");
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    }
  }

  async function create() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    try {
      await api.createUser(values);
      message.success(`已新增 ${values.name}`);
      setAdding(false);
      form.resetFields();
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    }
  }

  async function patch(user: ManagedUser, body: Record<string, unknown>) {
    try {
      await api.patchUser(user.id, body);
      message.success("已更新");
      onChanged();
    } catch (e) {
      message.error((e as Error).message, 8);
    }
  }

  async function resetPin() {
    const values = await pinForm.validateFields().catch(() => null);
    if (!values || !resetting) return;
    await patch(resetting, { password: values.password });
    setResetting(null);
    pinForm.resetFields();
  }

  return (
    <>
      <Card size="small" title="權限層級" style={{ marginBottom: 24 }}>
        <Text type="secondary" style={{ display: "block", marginBottom: 12 }}>
          三層權限的顯示名稱。<strong>這不是職位</strong> —— 職位（倉管／廠長／品管）填在下面每個人身上，
          兩個不同職位可以是同一個權限層級。
        </Text>
        <Row gutter={16}>
          {roles.map((r) => (
            <Col xs={24} md={8} key={r.code}>
              <Text type="secondary" style={{ fontSize: 12 }}>{SCOPE[r.code]}</Text>
              <Input
                defaultValue={r.label}
                onBlur={(e) => e.target.value.trim() && e.target.value !== r.label
                  && renameRole(r.code, e.target.value)}
              />
            </Col>
          ))}
        </Row>
      </Card>

      <Space style={{ marginBottom: 16 }}>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => setAdding(true)}>新增人員</Button>
        <Text type="secondary">改角色或停用會立即讓對方的登入失效，不會等到 12 小時後過期</Text>
      </Space>

      <Table
        rowKey="id"
        size="middle"
        pagination={false}
        dataSource={users}
        scroll={{ x: 1020 }}
        columns={[
          { title: "帳號", dataIndex: "username", width: 110, align: "center" as const,
            render: (v: string) => <Text code style={{ whiteSpace: "nowrap" }}>{v}</Text> },
          { title: "顯示名", dataIndex: "name", width: 100, align: "center" as const,
            render: (v: string) => <Text strong style={{ whiteSpace: "nowrap" }}>{v}</Text> },
          {
            title: "職位", dataIndex: "title", width: 180, align: "center" as const,
            render: (v: string | null, row) => (
              <CreatableSelect
                value={v ?? undefined}
                options={jobTitles}
                placeholder="未設定"
                addLabel="新增職位"
                category="job_title"
                style={{ width: 160 }}
                onAdded={onChanged}
                onChange={(next) => patch(row, { title: next })}
              />
            ),
          },
          {
            title: "權限層級", dataIndex: "role", width: 300, align: "center" as const,
            render: (v: string, row) => (
              <Select
                value={v}
                style={{ width: 290 }}
                options={ROLES}
                onChange={(next) => patch(row, { role: next })}
              />
            ),
          },
          {
            title: "狀態", dataIndex: "active", width: 170, align: "center" as const,
            render: (active: number, row) => (
              <Space>
                <Switch
                  size="small"
                  checked={Boolean(active)}
                  onChange={(next) => patch(row, { active: next })}
                />
                {active ? <Tag color="green">啟用中</Tag> : <Tag>已停用</Tag>}
                {row.id === me?.id && <Tag color="blue">你自己</Tag>}
              </Space>
            ),
          },
          {
            title: "密碼", width: 120,
            render: (_, row) => (
              <Button size="small" type="link" onClick={() => setResetting(row)}>重設</Button>
            ),
          },
        ]}
      />

      <Modal
        open={adding} title="新增人員" onCancel={() => setAdding(false)} onOk={create}
        okText="新增" cancelText="取消" width={520} destroyOnHidden
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="username" label="帳號"
            rules={[{ required: true, pattern: /^[a-zA-Z0-9._-]+$/, message: "英數字、點、底線、減號" }]}
          >
            <Input placeholder="例 kuo" autoComplete="off" />
          </Form.Item>
          <Form.Item
            name="name" label="顯示名（簽核用）"
            rules={[{ required: true, message: "請填顯示名" }]}
            extra="會寫進收貨單的記錄人與稽核軌跡"
          >
            <Input placeholder="例 郭" />
          </Form.Item>
          <Form.Item
            name="title" label="職位"
            extra="給人看的職稱（倉管／廠長／作業員），跟權限無關"
          >
            <CreatableSelect options={jobTitles} placeholder="選職位" addLabel="新增職位"
                             category="job_title" onAdded={onChanged} />
          </Form.Item>
          <Form.Item
            name="role" label="權限層級"
            rules={[{ required: true, message: "請選權限層級" }]}
            extra="決定他能做什麼。兩個不同職位可以是同一個層級"
          >
            <Select options={ROLES} placeholder="選層級" />
          </Form.Item>
          <Form.Item
            name="password" label="初始密碼"
            rules={[{ required: true, min: 8, message: "至少 8 個字元" }]}
            extra="對方第一次登入時會被要求改掉"
          >
            <Input.Password autoComplete="new-password" />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        open={Boolean(resetting)} title={`重設 ${resetting?.name} 的密碼`}
        onCancel={() => setResetting(null)} onOk={resetPin}
        okText="重設" cancelText="取消" width={420} destroyOnHidden
      >
        <Form form={pinForm} layout="vertical">
          <Form.Item
            name="password" label="新密碼"
            rules={[{ required: true, min: 8, message: "至少 8 個字元" }]}
            extra="對方下次登入會被要求改掉"
          >
            <Input.Password autoComplete="new-password" />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
