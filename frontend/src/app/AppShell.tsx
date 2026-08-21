"use client";

import {
  AlertOutlined,
  CameraOutlined,
  InboxOutlined,
  DatabaseOutlined,
  FileTextOutlined,
  SettingOutlined,
  ProfileOutlined,
  TableOutlined,
} from "@ant-design/icons";
import { Button, ConfigProvider, Layout, Menu, Result, Typography } from "antd";
import zhTW from "antd/locale/zh_TW";
import "dayjs/locale/zh-tw";
import Link from "next/link";
import { usePathname } from "next/navigation";
import AuthGate, { UserBadge, useAuth } from "@/components/AuthGate";

const { Sider, Content } = Layout;

// 基本資料 is first because it is first in the pipeline, not because it is
// settings: without a 型號 there is nothing to receive, and without machines
// and products there is nothing to record a draw against.
//
// `needs` hides what a role cannot use. Hiding is a convenience, not the
// control — the server enforces the same permission on every endpoint, because
// a menu item that is merely absent is one typed URL away from being present.
const NAV = [
  // The landing screen answers "what have we got, which box next" — the
  // question people walk up to the system with. Everyone sees it.
  { key: "/", icon: <DatabaseOutlined />, needs: null,
    label: <Link href="/">庫存總覽</Link> },
  { key: "/basics", icon: <TableOutlined />, needs: "item.manage",
    label: <Link href="/basics">1. 基本資料</Link> },
  { key: "/receiving", icon: <InboxOutlined />, needs: "lot.create",
    label: <Link href="/receiving">2. 收貨建批</Link> },
  { key: "/issue", icon: <CameraOutlined />, needs: "issue.create",
    label: <Link href="/issue">3. 領用登錄</Link> },
  { key: "/records", icon: <ProfileOutlined />, needs: null,
    label: <Link href="/records">4. 紀錄與追溯</Link> },
  { key: "/alerts", icon: <AlertOutlined />, needs: null,
    label: <Link href="/alerts">提醒</Link> },
  { key: "/logs", icon: <FileTextOutlined />, needs: "audit.read",
    label: <Link href="/logs">系統日誌</Link> },
  { key: "/settings", icon: <SettingOutlined />, needs: "dictionary.manage",
    label: <Link href="/settings">系統設定</Link> },
];

export default function AppShell({ children }: { children: React.ReactNode }) {
  // Locale only. The SoT specifies antd v6 defaults with no theme override,
  // so no `theme={}` here — the palette stays ChimesFlow-standard.
  return (
    <ConfigProvider locale={zhTW}>
      <AuthGate>
        <Shell>{children}</Shell>
      </AuthGate>
    </ConfigProvider>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const { can, user } = useAuth();
  const visible = NAV.filter((item) => !item.needs || can(item.needs));

  // Hiding the menu item is not enough: the URL still resolves, so someone
  // landing on it (a bookmark, a shared link, a role change mid-session) would
  // get a working-looking form that 403s only on submit. Say so up front, and
  // offer the way to the screen they can actually use.
  const current = NAV.find((item) => item.key === pathname);
  const denied = Boolean(current?.needs && !can(current.needs));
  const fallback = visible[0];

  return (
    <Layout style={{ minHeight: "100vh" }}>
      <Sider
        theme="dark"
        breakpoint="lg"
        collapsedWidth={64}
        width={220}
        // Sticky + flex column so the user badge sits at the bottom of the
        // VIEWPORT. It was absolutely positioned before, with no positioned
        // ancestor — so on a long page it landed at the bottom of the document
        // and the logout button was simply unreachable.
        style={{
          position: "sticky", top: 0, height: "100vh",
          display: "flex", flexDirection: "column",
        }}
      >
        <div style={{ padding: "20px 16px 12px" }}>
          <Typography.Text style={{ color: "#fff", fontSize: 18, fontWeight: 600 }}>
            Urd-WMS
          </Typography.Text>
          <div style={{ color: "#8c8c8c", fontSize: 12, marginTop: 2 }}>包材批次管理</div>
        </div>
        <Menu
          theme="dark"
          mode="inline"
          style={{ flex: 1, overflowY: "auto" }}
          selectedKeys={[pathname]}
          items={visible.map(({ key, icon, label }) => ({ key, icon, label }))}
        />
        <div style={{ padding: 16, borderTop: "1px solid rgba(255,255,255,0.1)" }}>
          <UserBadge />
        </div>
      </Sider>
      <Layout>
        <Content style={{ padding: 24, background: "#f5f5f5" }}>
          {denied ? (
            <Result
              status="403"
              title="這個畫面不開放給你的身分"
              subTitle={`${user?.name}（${user?.role_label}）沒有這個畫面的權限。需要別的權限請找系統管理者。`}
              extra={fallback ? <Link href={fallback.key}><Button type="primary">回到能用的畫面</Button></Link> : null}
            />
          ) : (
            children
          )}
        </Content>
      </Layout>
    </Layout>
  );
}
