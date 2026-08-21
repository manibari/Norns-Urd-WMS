"use client";

/**
 * One dictionary category: the values a dropdown offers.
 *
 * Retiring a value deactivates rather than deletes it. Historical records point
 * at these values, and a traceability report that renders a blank where a
 * machine used to be is worse than one naming a machine that no longer exists.
 * Re-adding a retired value revives the original row for the same reason.
 */

import { PlusOutlined } from "@ant-design/icons";
import { Button, Empty, Input, Popconfirm, Space, Switch, Table, Tag, Tooltip, Typography, message } from "antd";
import { useState } from "react";
import { api, type DictEntry } from "@/lib/api";

const { Text } = Typography;

export default function DictionaryTable({
  category, label, entries, onChanged,
}: {
  category: string;
  label: string;
  entries: DictEntry[];
  onChanged: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  async function add() {
    const value = draft.trim();
    if (!value) return;
    setBusy(true);
    try {
      const res = await api.addDictEntry(category, value);
      message.success(res.revived ? `已重新啟用「${value}」` : `已新增「${value}」`);
      setDraft("");
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function toggle(entry: DictEntry, active: boolean) {
    setBusy(true);
    try {
      await api.patchDictEntry(entry.id, { active });
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function remove(entry: DictEntry) {
    setBusy(true);
    try {
      await api.deleteDictEntry(entry.id);
      message.success(`已刪除「${entry.value}」`);
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function rename(entry: DictEntry, value: string) {
    if (!value.trim() || value.trim() === entry.value) return;
    setBusy(true);
    try {
      await api.patchDictEntry(entry.id, { value: value.trim() });
      message.success("已更名");
      onChanged();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Space.Compact style={{ width: 420, marginBottom: 16 }}>
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onPressEnter={add}
          placeholder={`新增${label}`}
          disabled={busy}
        />
        <Button type="primary" icon={<PlusOutlined />} onClick={add} loading={busy}>
          新增
        </Button>
      </Space.Compact>

      <Table
        rowKey="id"
        size="middle"
        pagination={false}
        dataSource={entries}
        locale={{ emptyText: <Empty description={`尚無${label}`} image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
        columns={[
          {
            title: label,
            dataIndex: "value",
            align: "center" as const,
            render: (v: string, row: DictEntry) => (
              <Input
                defaultValue={v}
                variant="borderless"
                style={{ paddingLeft: 0, maxWidth: 320 }}
                onBlur={(e) => rename(row, e.target.value)}
                disabled={!row.active}
              />
            ),
          },
          {
            title: "狀態",
            dataIndex: "active",
            width: 190,
            align: "center" as const,
            render: (active: number, row: DictEntry) => (
              <Space>
                <Switch
                  size="small"
                  checked={Boolean(active)}
                  onChange={(next) => toggle(row, next)}
                  disabled={busy}
                />
                {active ? <Tag color="green">啟用中</Tag> : <Tag>已停用</Tag>}
              </Space>
            ),
          },
          {
            title: "",
            width: 80,
            align: "center" as const,
            render: (_, row: DictEntry) => (
              <Popconfirm
                title={`刪除「${row.value}」？`}
                description="既有紀錄不受影響（存的是文字，不是參照）。只是不再出現在選單。"
                okText="刪除" cancelText="取消" okButtonProps={{ danger: true }}
                onConfirm={() => remove(row)}
              >
                <Button size="small" type="text" danger disabled={busy}>刪除</Button>
              </Popconfirm>
            ),
          },
        ]}
      />
      <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
        <strong>停用</strong>：不再出現在下拉選單，但保留在清單裡，隨時可以再啟用 —— 用在「這個先不用了」。
        <br />
        <strong>刪除</strong>：整個移除。既有紀錄不受影響（存的是當時選的文字，不是參照），
        所以刪掉「弘東京」不會讓去年的收貨單變空白 —— 用在「這一筆本來就是打錯的」。
      </Text>
    </>
  );
}
