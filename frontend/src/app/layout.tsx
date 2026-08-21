import type { Metadata } from "next";
import { AntdRegistry } from "@ant-design/nextjs-registry";
import AppShell from "./AppShell";

export const metadata: Metadata = {
  title: "Urd-WMS 包材批次管理",
  description: "拍箱子取代抄表單 — FIFO 稽核、追溯、提醒",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="zh-TW">
      <body style={{ margin: 0, fontFamily: "Arial, Helvetica, sans-serif" }}>
        <AntdRegistry>
          <AppShell>{children}</AppShell>
        </AntdRegistry>
      </body>
    </html>
  );
}
