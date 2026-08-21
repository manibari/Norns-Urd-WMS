"use client";

/**
 * Login gate.
 *
 * Username and password. An account created by an admin is flagged
 * `must_change`, so the first thing it sees is the change-password form —
 * otherwise the admin knows everyone's password indefinitely, and the signature
 * on a receiving line stops meaning "this person".
 */

import { LockOutlined, LogoutOutlined, UserOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Form, Input, Modal, Space, Typography, message } from "antd";
import { createContext, useContext, useEffect, useState } from "react";
import { api, token as tokenStore, type SessionUser } from "@/lib/api";

const { Title, Text } = Typography;

type AuthValue = {
  user: SessionUser | null;
  can: (permission: string) => boolean;
  logout: () => void;
  openPasswordChange: () => void;
};

const AuthContext = createContext<AuthValue>({
  user: null, can: () => false, logout: () => {}, openPasswordChange: () => {},
});

export const useAuth = () => useContext(AuthContext);

export default function AuthGate({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<SessionUser | null>(null);
  const [checking, setChecking] = useState(true);
  const [busy, setBusy] = useState(false);
  const [changing, setChanging] = useState(false);
  const [form] = Form.useForm();
  const [pwForm] = Form.useForm();

  useEffect(() => {
    (async () => {
      if (tokenStore.get()) {
        try {
          const me = await api.me();
          setUser(me);
          if (me.must_change) setChanging(true);
        } catch {
          tokenStore.clear();
        }
      }
      setChecking(false);
    })();
  }, []);

  async function submit() {
    const values = await form.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      const res = await api.login(values.username, values.password);
      tokenStore.set(res.token);
      setUser(res.user);
      form.resetFields();
      if (res.user.must_change) {
        setChanging(true);
        message.warning("這是別人幫你設的密碼，請改成只有你知道的");
      }
    } catch (e) {
      message.error((e as Error).message);
      form.setFieldValue("password", "");
    } finally {
      setBusy(false);
    }
  }

  async function changePassword() {
    const values = await pwForm.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      await api.changePassword(values.current_password, values.new_password);
      message.success("密碼已更改，請重新登入");
      // The server drops every session for this account on a password change,
      // including this one — so send the user back to the login form rather
      // than leaving them holding a token that no longer works.
      tokenStore.clear();
      setUser(null);
      setChanging(false);
      pwForm.resetFields();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function logout() {
    try {
      await api.logout();
    } catch {
      // Clearing locally is what matters; an orphaned server session expires.
    }
    tokenStore.clear();
    setUser(null);
  }

  if (checking) return null;

  if (!user) {
    return (
      <div style={{ minHeight: "100vh", display: "flex", alignItems: "center",
                    justifyContent: "center", background: "#f5f5f5", padding: 24 }}>
        <Card style={{ width: 400 }}>
          <Title level={3} style={{ marginTop: 0 }}>Urd-WMS</Title>
          <Text type="secondary" style={{ display: "block", marginBottom: 24 }}>包材批次管理</Text>
          <Form form={form} layout="vertical" onFinish={submit}>
            <Form.Item name="username" label="帳號" rules={[{ required: true, message: "請填帳號" }]}>
              <Input size="large" prefix={<UserOutlined />} autoComplete="username" autoFocus />
            </Form.Item>
            <Form.Item name="password" label="密碼" rules={[{ required: true, message: "請填密碼" }]}>
              <Input.Password size="large" prefix={<LockOutlined />} autoComplete="current-password" />
            </Form.Item>
            <Button type="primary" size="large" htmlType="submit" loading={busy} block>
              登入
            </Button>
          </Form>
        </Card>
      </div>
    );
  }

  return (
    <AuthContext.Provider
      value={{
        user,
        can: (p) => user.permissions.includes(p),
        logout,
        openPasswordChange: () => setChanging(true),
      }}
    >
      {children}
      <Modal
        open={changing}
        title="更改密碼"
        onOk={changePassword}
        onCancel={() => setChanging(false)}
        okText="更改" cancelText="稍後"
        confirmLoading={busy}
        // A forced change has no way out: cancelling would leave the account on
        // a password its creator knows.
        closable={!user.must_change}
        mask={{ closable: !user.must_change }}
        cancelButtonProps={{ style: user.must_change ? { display: "none" } : undefined }}
        width={440}
        destroyOnHidden
      >
        {user.must_change && (
          <Alert
            type="warning"
            title="這組密碼是別人幫你設的"
            description="改成只有你知道的密碼。收貨單上的「記錄人」會寫你的名字，那個簽名要能代表你本人。"
            style={{ marginBottom: 16 }}
          />
        )}
        <Form form={pwForm} layout="vertical">
          <Form.Item name="current_password" label="目前密碼" rules={[{ required: true }]}>
            <Input.Password autoComplete="current-password" />
          </Form.Item>
          <Form.Item
            name="new_password"
            label="新密碼"
            rules={[{ required: true, min: 8, message: "至少 8 個字元" }]}
            extra="至少 8 個字元。沒有大小寫或符號的規定 —— 那種規定只會逼出 Password1! 和便利貼"
          >
            <Input.Password autoComplete="new-password" />
          </Form.Item>
          <Form.Item
            name="confirm"
            label="再輸入一次"
            dependencies={["new_password"]}
            rules={[
              { required: true },
              ({ getFieldValue }) => ({
                validator: (_, value) =>
                  !value || getFieldValue("new_password") === value
                    ? Promise.resolve()
                    : Promise.reject(new Error("兩次輸入不一致")),
              }),
            ]}
          >
            <Input.Password autoComplete="new-password" />
          </Form.Item>
        </Form>
      </Modal>
    </AuthContext.Provider>
  );
}

export function UserBadge() {
  const { user, logout, openPasswordChange } = useAuth();
  if (!user) return null;
  return (
    <Space>
      <Space orientation="vertical" size={0} style={{ lineHeight: 1.3, cursor: "pointer" }}
             onClick={openPasswordChange}>
        <Text style={{ color: "#fff" }}>{user.name}</Text>
        {/* The job title, not the permission tier — "倉管" is what someone
            recognises themselves as; "manager" is a server-side concept. */}
        <Text style={{ color: "#8c8c8c", fontSize: 12 }}>{user.title ?? user.role_label}</Text>
      </Space>
      <Button type="text" icon={<LogoutOutlined />} style={{ color: "#8c8c8c" }} onClick={logout} />
    </Space>
  );
}
