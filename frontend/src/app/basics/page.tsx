"use client";

/**
 * 基本資料 — every dropdown in the app is fed from here.
 *
 * First in the nav because it is first in the pipeline: without a 型號 and its
 * metres rate, receiving cannot record a delivery; without machines and
 * products, issuing cannot record a draw. Configuration is not a settings
 * screen you visit once, it is step zero.
 *
 * This is also US-11 (廠別配置) starting to exist for real: a second factory
 * changes these tables, not the code.
 *
 * The 型號 tab additionally carries the metres-per-box table.
 *
 * Suppliers label film by length and some deliveries are counted that way, so
 * receiving needs a rate to convert with. Stock itself stays in boxes: one box
 * is one roll, a draw deducts one, and there is no partial consumption
 * (requirement Q6). Keeping stock in metres would make "half a roll left"
 * representable and quietly undo that decision — so this table feeds conversion
 * and display, not the ledger.
 *
 * An unset rate is a legitimate state, not missing data: plenty of items are
 * only ever counted in boxes. Receiving refuses a metre entry for those rather
 * than inventing a rate.
 */

import { Alert, Card, Empty, InputNumber, Table, Tabs, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import DictionaryTable from "@/components/DictionaryTable";
import { api, type Dictionary, type Item } from "@/lib/api";

const { Title, Text } = Typography;

export default function BasicsPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [saving, setSaving] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [i, d] = await Promise.all([api.items(), api.dictionary(true)]);
      setItems(i);
      setDict(d);
    } catch (e) {
      message.error((e as Error).message);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  async function save(code: string, field: string, value: number | null) {
    setSaving(code);
    try {
      await api.patchItem(code, { [field]: value });
      message.success("已更新");
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setSaving(null);
    }
  }

  const unset = items.filter((i) => !i.meters_per_box).length;

  const itemsTab = (
    <>
      <Alert
        type="info"
        title="米數是換算用的，庫存單位仍然是箱"
        description="驗收單上的數量是米，這張表就是把它換算成箱的依據（例：T6050BSW 一箱 600 米）。但庫存記帳仍以箱為單位 —— 一箱＝一捲，領用一次扣一箱，不做部分扣量。除不盡的餘數會明白告訴你，不會四捨五入吃掉。"
        style={{ marginBottom: 24 }}
      />

      <Card
        title="型號對照表"
        extra={unset > 0 ? <Text type="secondary">{unset} 項未設定米數（僅能用米數以外的方式收貨）</Text> : null}
      >
        <Table
          rowKey="item_code"
          dataSource={items}
          pagination={false}
          size="middle"
          loading={Boolean(saving)}
          scroll={{ x: 1180 }}
          locale={{ emptyText: <Empty description="尚無品項，請先去收貨建批" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          columns={[
            {
              title: "型號",
              dataIndex: "item_code",
              width: 120,
              render: (v: string) => <Text strong style={{ whiteSpace: "nowrap" }}>{v}</Text>,
            },
            { title: "原物料名稱", dataIndex: "name", width: 190 },
            { title: "規格", dataIndex: "spec", width: 140, render: (v) => v ?? "—" },
            { title: "廠商", dataIndex: "supplier", width: 110, render: (v) => v ?? "—" },
            {
              title: "箱上完整料號",
              dataIndex: "supplier_code",
              width: 230,
              render: (v: string | null) =>
                v ? <Text code>{v}</Text> : <Text type="secondary">未登記（辨識無法對映）</Text>,
            },
            {
              title: "每箱米數（米）",
              dataIndex: "meters_per_box",
              width: 200,
              render: (v: number | null, row: Item) => (
                <InputNumber
                  min={1}
                  value={v ?? undefined}
                  placeholder="未設定"
                  style={{ width: 160 }}
                  onBlur={(e) => {
                    const next = e.target.value ? Number(e.target.value) : null;
                    if (next !== v && next && next > 0) save(row.item_code, "meters_per_box", next);
                  }}
                />
              ),
            },
            {
              title: "在庫",
              dataIndex: "on_hand",
              align: "right" as const,
              width: 150,
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
              title: "保存期限（天）",
              dataIndex: "shelf_life_days",
              align: "right" as const,
              render: (v: number | null, row: Item) => (
                <InputNumber
                  min={1}
                  value={v ?? undefined}
                  placeholder="不提醒"
                  style={{ width: 140 }}
                  onBlur={(e) => {
                    const next = e.target.value ? Number(e.target.value) : null;
                    if (next !== v && next && next > 0) save(row.item_code, "shelf_life_days", next);
                  }}
                />
              ),
            },
            {
              title: "安全水位",
              dataIndex: "safety_stock",
              align: "right" as const,
              render: (v: number) => (v > 0 ? <Tag color="blue">{v} 箱</Tag> : <Text type="secondary">未設</Text>),
            },
          ]}
        />
      </Card>
    </>
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
            { key: "items", label: `型號與米數（${items.length}）`, children: itemsTab },
            ...Object.entries(dict?.categories ?? {}).map(([key, label]) => {
              const entries = dict?.entries[key] ?? [];
              const activeCount = entries.filter((e) => e.active).length;
              return {
                key,
                label: `${label}（${activeCount}）`,
                children: (
                  <DictionaryTable category={key} label={label} entries={entries} onChanged={load} />
                ),
              };
            }),
          ]}
        />
      </Card>
    </>
  );
}
