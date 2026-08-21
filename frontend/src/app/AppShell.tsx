"use client";

import {
  AlertOutlined,
  CameraOutlined,
  InboxOutlined,
  ProfileOutlined,
  TableOutlined,
} from "@ant-design/icons";
import { ConfigProvider, Layout, Menu, Typography } from "antd";
import zhTW from "antd/locale/zh_TW";
import "dayjs/locale/zh-tw";
import Link from "next/link";
import { usePathname } from "next/navigation";

const { Sider, Content } = Layout;

// Order is the pipeline, not a menu preference: a lot has to be received before
// it can be issued, and a draw has to happen before there is anything to trace.
// Landing on 收貨建批 also means a fresh install opens on the only screen that
// can do anything — the issuing screen has an empty candidate set until stock exists.
const NAV = [
  { key: "/", icon: <InboxOutlined />, label: <Link href="/">1. 收貨建批</Link> },
  { key: "/issue", icon: <CameraOutlined />, label: <Link href="/issue">2. 領用登錄</Link> },
  { key: "/records", icon: <ProfileOutlined />, label: <Link href="/records">3. 紀錄與追溯</Link> },
  { key: "/alerts", icon: <AlertOutlined />, label: <Link href="/alerts">提醒</Link> },
  // Master data, not a pipeline step — hence unnumbered and last.
  { key: "/items", icon: <TableOutlined />, label: <Link href="/items">品項與米數</Link> },
];

export default function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  return (
    // Locale only. The SoT specifies antd v6 defaults with no theme override,
    // so no `theme={}` here — the palette stays ChimesFlow-standard.
    <ConfigProvider locale={zhTW}>
    <Layout style={{ minHeight: "100vh" }}>
      <Sider theme="dark" breakpoint="lg" collapsedWidth={64} width={220}>
        <div style={{ padding: "20px 16px 12px" }}>
          <Typography.Text style={{ color: "#fff", fontSize: 18, fontWeight: 600 }}>
            Urd-WMS
          </Typography.Text>
          <div style={{ color: "#8c8c8c", fontSize: 12, marginTop: 2 }}>包材批次管理</div>
        </div>
        <Menu theme="dark" mode="inline" selectedKeys={[pathname]} items={NAV} />
      </Sider>
      <Layout>
        <Content style={{ padding: 24, background: "#f5f5f5" }}>{children}</Content>
      </Layout>
    </Layout>
    </ConfigProvider>
  );
}
