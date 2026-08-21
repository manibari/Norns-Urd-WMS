"use client";

/**
 * System log.
 *
 * Every write in the app lands in audit_log: logins, receiving, corrections,
 * deletions, dictionary and role edits, account changes, draws and overrides.
 * US-1 requires corrections to be recorded rather than silent, and a record
 * nobody can read is silent in every way that matters — so it gets a screen of
 * its own rather than living as a tab someone has to know about.
 *
 * Corrections render before → after. "Someone changed the receipt date" says
 * nothing without the old value, and the receipt date is the FIFO sort key.
 */

import { Card, Empty, Input, Select, Space, Table, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type AuditEntry } from "@/lib/api";

const { Title, Text } = Typography;

const ACTION_LABELS: Record<string, string> = {
  "auth.login": "登入",
  "auth.password_change": "更改密碼",
  "lot.create": "收貨建批",
  "lot.update": "批次修正",
  "lot.delete": "批次刪除",
  "item.create": "新增型號",
  "item.update": "型號修正",
  "item.delete": "型號刪除",
  "dictionary.create": "選項新增",
  "dictionary.update": "選項修改",
  "dictionary.revive": "選項啟用",
  "role.rename": "角色改名",
  "user.create": "新增人員",
  "user.update": "人員異動",
  "scan.posted": "領用登錄",
  "scan.blocked_fifo": "非 FIFO 擋下",
  "scan.blocked_unreadable": "待人工補批次",
  "scan.overridden": "覆核放行",
  "seed.run": "初始資料",
};

const GROUPS: { value: string; label: string }[] = [
  { value: "", label: "全部" },
  { value: "lot", label: "批次" },
  { value: "item", label: "型號" },
  { value: "scan", label: "領用" },
  { value: "auth", label: "登入／密碼" },
  { value: "user", label: "人員" },
  { value: "dictionary", label: "選項" },
  { value: "role", label: "角色" },
];

export default function LogsPage() {
  const [rows, setRows] = useState<AuditEntry[]>([]);
  const [group, setGroup] = useState("");
  const [keyword, setKeyword] = useState("");
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (prefix: string) => {
    setLoading(true);
    try {
      setRows(await api.audit(prefix || undefined));
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(group); }, [group, load]);

  const filtered = useMemo(() => {
    const needle = keyword.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter((r) =>
      r.actor.toLowerCase().includes(needle)
      || (ACTION_LABELS[r.action] ?? r.action).toLowerCase().includes(needle)
      || JSON.stringify(r.detail).toLowerCase().includes(needle));
  }, [rows, keyword]);

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>系統日誌</Title>
      <Text type="secondary" style={{ display: "block", marginBottom: 16 }}>
        所有寫入操作都留在這裡：登入、收貨、修正、刪除、選項與角色異動、領用與覆核。修正會顯示改動前後的值。
      </Text>

      <Card>
        <Space style={{ marginBottom: 16 }} wrap>
          <Select
            value={group}
            onChange={setGroup}
            options={GROUPS}
            style={{ width: 160 }}
          />
          <Input.Search
            placeholder="搜尋操作者、動作、內容"
            allowClear
            style={{ width: 320 }}
            onChange={(e) => setKeyword(e.target.value)}
          />
          <Text type="secondary">{filtered.length} 筆</Text>
        </Space>

        <Table
          rowKey="id"
          size="middle"
          loading={loading}
          pagination={{ pageSize: 30, showSizeChanger: false }}
          dataSource={filtered}
          locale={{ emptyText: <Empty description="沒有符合的紀錄" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          columns={[
            { title: "時間", dataIndex: "at", width: 165, align: "center" as const,
              render: (v: string) => v.slice(0, 19).replace("T", " ") },
            { title: "操作者", dataIndex: "actor", width: 120, align: "center" as const },
            {
              title: "動作", dataIndex: "action", width: 140, align: "center" as const,
              render: (v: string) => <Tag>{ACTION_LABELS[v] ?? v}</Tag>,
            },
            {
              title: "內容", dataIndex: "detail",
              render: (detail: Record<string, unknown>) => {
                const before = detail.before as Record<string, unknown> | undefined;
                const after = detail.after as Record<string, unknown> | undefined;
                if (before && after) {
                  return (
                    <Space orientation="vertical" size={0}>
                      {Object.keys(after).map((k) => (
                        <span key={k}>
                          <Text type="secondary">{k}：</Text>
                          <Text delete type="secondary">{String(before[k] ?? "—")}</Text>
                          {" → "}
                          <Text strong>{String(after[k] ?? "—")}</Text>
                        </span>
                      ))}
                    </Space>
                  );
                }
                return (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {Object.entries(detail).map(([k, v]) => `${k}=${JSON.stringify(v)}`).join("　")}
                  </Text>
                );
              },
            },
          ]}
        />
      </Card>
    </>
  );
}
