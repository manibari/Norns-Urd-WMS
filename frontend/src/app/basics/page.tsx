"use client";

/**
 * 基本資料 — every dropdown in the app is fed from here.
 *
 * First in the nav because it is first in the pipeline: without a 型號 and its
 * metres rate, receiving cannot record a delivery; without machines and
 * products, issuing cannot record a draw. Configuration is not a settings
 * screen you visit once, it is step zero.
 *
 * This is US-11 (廠別配置) starting to exist for real: a second factory changes
 * these tables, not the code.
 *
 * A 型號 carries its 廠商 / 原物料名稱 / 規格 rather than those being picked
 * separately at receiving — T7320BC IS 臺灣希悅爾's 高阻氧拉伸膜 340mm x 900M,
 * and letting someone combine them freely allows combinations that do not exist.
 */

import { PlusOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Empty, Form, Input, InputNumber, Modal, Popconfirm,
  Row, Select, Space, Switch, Table, Tabs, Tag, Tooltip, Typography, message,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/components/AuthGate";
import CreatableSelect from "@/components/CreatableSelect";
import DictionaryTable from "@/components/DictionaryTable";
import { api, type Dictionary, type Item, type SessionUser } from "@/lib/api";

const { Title, Text } = Typography;

type ManagedUser = SessionUser & { active: number; created_at: string };

export default function BasicsPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [users, setUsers] = useState<ManagedUser[]>([]);
  const [roles, setRoles] = useState<{ code: string; label: string; default_label: string }[]>([]);
  // 廠商 / 原物料名稱 / 規格 are attributes of a 型號, so their options are the
  // distinct values already on the item master — not a separate table that
  // could disagree with it.
  const [options, setOptions] = useState<{ supplier: string[]; material_name: string[]; spec: string[] }>(
    { supplier: [], material_name: [], spec: [] },
  );
  const [editing, setEditing] = useState<Item | "new" | null>(null);
  const [busy, setBusy] = useState(false);
  const [form] = Form.useForm();
  const { can, user: me } = useAuth();

  const load = useCallback(async () => {
    const [i, d, o, r] = await Promise.allSettled([
      api.items(), api.dictionary(true), api.itemOptions(), api.roles(),
    ]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`型號載入失敗：${i.reason?.message}`);
    if (d.status === "fulfilled") setDict(d.value);
    else message.error(`選項載入失敗：${d.reason?.message}`);
    if (o.status === "fulfilled") setOptions(o.value);
    if (r.status === "fulfilled") setRoles(r.value);
    if (can("user.manage")) {
      try {
        setUsers(await api.users());
      } catch (e) {
        message.error(`人員載入失敗：${(e as Error).message}`);
      }
    }
  }, [can]);

  useEffect(() => { load(); }, [load]);

  async function saveItem() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      if (editing === "new") {
        await api.createItem(values);
        message.success(`已新增型號 ${values.item_code}`);
      } else if (editing) {
        await api.patchItem(editing.item_code, values);
        message.success("已更新");
      }
      setEditing(null);
      form.resetFields();
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function removeItem(code: string) {
    try {
      await api.deleteItem(code);
      message.success("已刪除");
      load();
    } catch (e) {
      message.error((e as Error).message, 10);
    }
  }

  const unset = items.filter((i) => !i.meters_per_box).length;

  const itemsTab = (
    <>
      <Alert
        type="info"
        title="一個型號就是「某廠商的某原物料的某規格」"
        description="所以廠商／原物料名稱／規格沒有各自的清單 —— 它們是這張表的欄位，要改就點那一列的「編輯」。收貨時只選型號，這三欄會自動帶出。米數是換算用的：驗收單數量填米，這張表換成箱；庫存記帳仍以箱為單位，一箱＝一捲，不做部分扣量。"
        style={{ marginBottom: 24 }}
      />

      <Card
        title="型號對照表"
        extra={
          <Space>
            {unset > 0 && <Text type="secondary">{unset} 項未設米數</Text>}
            {can("item.manage") && (
              <Button
                type="primary"
                icon={<PlusOutlined />}
                onClick={() => { setEditing("new"); form.resetFields(); }}
              >
                新增型號
              </Button>
            )}
          </Space>
        }
      >
        <Table
          rowKey="item_code"
          dataSource={items}
          pagination={false}
          size="middle"
          scroll={{ x: 1200 }}
          locale={{ emptyText: <Empty description="尚無型號" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          columns={[
            {
              title: "型號", dataIndex: "item_code", width: 120,
              render: (v: string) => <Text strong style={{ whiteSpace: "nowrap" }}>{v}</Text>,
            },
            { title: "原物料名稱", dataIndex: "name", width: 190 },
            { title: "規格", dataIndex: "spec", width: 140, render: (v) => v ?? "—" },
            { title: "廠商", dataIndex: "supplier", width: 110, render: (v) => v ?? "—" },
            {
              title: "箱上完整料號", dataIndex: "supplier_code", width: 220,
              render: (v: string | null) =>
                v ? <Text code>{v}</Text> : <Text type="secondary">未登記（辨識無法對映）</Text>,
            },
            {
              title: "每箱米數", dataIndex: "meters_per_box", width: 100, align: "right" as const,
              render: (v: number | null) => (v ? `${v.toLocaleString()} 米` : <Text type="secondary">未設</Text>),
            },
            {
              title: "在庫", dataIndex: "on_hand", width: 140, align: "right" as const,
              render: (v: number, row: Item) => (
                <span>
                  {v} 箱
                  {row.on_hand_m != null && (
                    <Text type="secondary">（{row.on_hand_m.toLocaleString()} 米）</Text>
                  )}
                </span>
              ),
            },
            {
              title: "保存期限", dataIndex: "shelf_life_days", width: 100, align: "right" as const,
              render: (v: number | null) => (v ? `${v} 天` : <Text type="secondary">不提醒</Text>),
            },
            {
              title: "安全水位", dataIndex: "safety_stock", width: 90, align: "right" as const,
              render: (v: number) => (v > 0 ? <Tag color="blue">{v} 箱</Tag> : <Text type="secondary">未設</Text>),
            },
            {
              title: "管理", width: 130, fixed: "right" as const, hidden: !can("item.manage"),
              render: (_, row: Item) => (
                <Space size={4}>
                  <Button
                    size="small" type="link"
                    onClick={() => { setEditing(row); form.setFieldsValue(row); }}
                  >
                    編輯
                  </Button>
                  {row.on_hand > 0 || row.open_lots > 0 || row.rejected_qty > 0 ? (
                    <Tooltip title="已有進貨或領用紀錄，刪掉那些紀錄會失去對應。不再使用請停止收貨。">
                      <Button size="small" type="link" disabled>刪除</Button>
                    </Tooltip>
                  ) : (
                    <Popconfirm
                      title={`刪除 ${row.item_code}？`}
                      okText="刪除" cancelText="取消" okButtonProps={{ danger: true }}
                      onConfirm={() => removeItem(row.item_code)}
                    >
                      <Button size="small" type="link" danger>刪除</Button>
                    </Popconfirm>
                  )}
                </Space>
              ),
            },
          ]}
        />
      </Card>
    </>
  );

  const usersTab = (
    <UsersPanel
      users={users}
      me={me}
      roles={roles}
      jobTitles={(dict?.entries.job_title ?? []).filter((e) => e.active).map((e) => e.value)}
      onChanged={load}
    />
  );

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>基本資料</Title>
      <Text type="secondary" style={{ display: "block", marginBottom: 16 }}>
        收貨與領用畫面的每一個下拉選單都從這裡來。換一家工廠就是換這幾張表的內容，不用改程式。
      </Text>

      <Card>
        <Tabs
          items={[
            { key: "items", label: `型號（${items.length}）`, children: itemsTab },
            ...Object.entries(dict?.categories ?? {}).map(([key, label]) => {
              const entries = dict?.entries[key] ?? [];
              return {
                key,
                label: `${label}（${entries.filter((e) => e.active).length}）`,
                children: (
                  <DictionaryTable category={key} label={label} entries={entries} onChanged={load} />
                ),
              };
            }),
            ...(can("user.manage")
              ? [{ key: "users", label: `人員（${users.filter((u) => u.active).length}）`, children: usersTab }]
              : []),
          ]}
        />
      </Card>

      <Modal
        open={Boolean(editing)}
        title={editing === "new" ? "新增型號" : `編輯型號 ${editing && editing !== "new" ? editing.item_code : ""}`}
        onCancel={() => { setEditing(null); form.resetFields(); }}
        onOk={saveItem}
        confirmLoading={busy}
        okText="儲存" cancelText="取消" width={760} destroyOnHidden
      >
        <Form form={form} layout="vertical">
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item
                name="item_code" label="型號"
                rules={[{ required: true, message: "請填型號" }]}
                extra={editing === "new" ? "驗收單上填的那個代號" : "型號不可修改"}
              >
                <Input disabled={editing !== "new"} placeholder="例 T6050BSW" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="name" label="原物料名稱" rules={[{ required: true, message: "請填名稱" }]}>
                <CreatableSelect options={options.material_name} placeholder="選或直接輸入"
                                 addLabel="新增名稱" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="spec" label="規格">
                <CreatableSelect options={options.spec} placeholder="選或直接輸入"
                                 addLabel="新增規格" />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="supplier" label="廠商">
                <CreatableSelect options={options.supplier} placeholder="選或直接輸入"
                                 addLabel="新增廠商" />
              </Form.Item>
            </Col>
            <Col span={16}>
              <Form.Item
                name="supplier_code" label="箱上完整料號"
                extra="影像辨識讀到的是這個，用來對回型號。不填則辨識時要人工選型號"
              >
                <Input placeholder="例 2003.T7320BC-340X900-P1" />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col span={8}>
              <Form.Item name="meters_per_box" label="每箱米數" extra="設了才能用米數收貨">
                <InputNumber min={1} step={100} style={{ width: "100%" }} placeholder="例 600" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="shelf_life_days" label="保存期限（天）" extra="留空則不發效期提醒">
                <InputNumber min={1} style={{ width: "100%" }} placeholder="例 540" />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="safety_stock" label="安全水位（箱）" extra="低於此值發低水位提醒">
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
          </Row>
        </Form>
      </Modal>
    </>
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
          { title: "帳號", dataIndex: "username", width: 110,
            render: (v: string) => <Text code style={{ whiteSpace: "nowrap" }}>{v}</Text> },
          { title: "顯示名", dataIndex: "name", width: 100,
            render: (v: string) => <Text strong style={{ whiteSpace: "nowrap" }}>{v}</Text> },
          {
            title: "職位", dataIndex: "title", width: 180,
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
            title: "權限層級", dataIndex: "role", width: 300,
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
            title: "狀態", dataIndex: "active", width: 170,
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
