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
import { useRowDrag } from "@/components/DraggableRows";
import { NumberCell, TextCell } from "@/components/EditableCell";
import { useColumnWidths } from "@/components/resizable";
import DictionaryTable from "@/components/DictionaryTable";
import { api, type Dictionary, type Item } from "@/lib/api";

const { Title, Text } = Typography;

export default function BasicsPage() {
  const [items, setItems] = useState<Item[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  // 廠商 / 原物料名稱 / 規格 are attributes of a 型號, so their options are the
  // distinct values already on the item master — not a separate table that
  // could disagree with it.
  const [options, setOptions] = useState<{ supplier: string[]; material_name: string[]; spec: string[] }>(
    { supplier: [], material_name: [], spec: [] },
  );
  const [busy, setBusy] = useState(false);
  const [draftForm] = Form.useForm();
  const { can } = useAuth();
  const itemCols = useColumnWidths("items");
  const rowDrag = useRowDrag(items, async (orderedIds) => {
    // Optimistic: the list jumps immediately, then the server confirms. A
    // half-second lag on a drag makes it feel like the drop did not take.
    setItems((prev) => orderedIds.map((id) => prev.find((i) => i.id === id)!).filter(Boolean));
    try {
      await api.reorderItems(orderedIds);
    } catch (e) {
      message.error(`順序沒存起來：${(e as Error).message}`);
      load();
    }
  });

  const load = useCallback(async () => {
    const [i, d, o] = await Promise.allSettled([
      api.items(), api.dictionary(true), api.itemOptions(),
    ]);
    if (i.status === "fulfilled") setItems(i.value);
    else message.error(`型號載入失敗：${i.reason?.message}`);
    if (d.status === "fulfilled") setDict(d.value);
    else message.error(`選項載入失敗：${d.reason?.message}`);
    if (o.status === "fulfilled") setOptions(o.value);
  }, []);

  useEffect(() => { load(); }, [load]);

  /** Patch one field of one row, updating just that row rather than reloading. */
  const dictValues = (category: string) =>
    (dict?.entries[category] ?? []).filter((e) => e.active).map((e) => e.value);

  async function patchField(item: Item, field: string, value: unknown) {
    try {
      await api.patchItem(item.id, { [field]: value });
      const fresh = await api.items();
      setItems(fresh);
      // Supplier / name / spec options are derived from the master, so a change
      // here can introduce a new option elsewhere.
      api.itemOptions().then(setOptions).catch(() => undefined);
    } catch (e) {
      message.error((e as Error).message, 6);
      // Reload so the cell snaps back to the stored value rather than showing
      // an edit the server rejected.
      api.items().then(setItems).catch(() => undefined);
    }
  }

  async function addItem() {
    const values = await draftForm.validateFields().catch(() => null);
    if (!values) return;
    setBusy(true);
    try {
      await api.createItem({ ...values, model: values.model || null });
      message.success(`已新增 ${values.model || values.name}`);
      draftForm.resetFields();
      load();
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function removeItem(item: Item) {
    try {
      await api.deleteItem(item.id);
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
        title="必填的是原物料名稱，型號可以沒有"
        description="驗收單上「脫氧劑」那列就沒有型號，所以型號不是識別。廠商／規格也是這張表的欄位，不另開清單 —— 收貨時選了品項就會自動帶出。「每箱數量」是驗收單數量換算成箱的依據，單位隨品項而定（膜是米、袋是張、劑是包），所以只填數字。⚠️ 只有有型號（或登記過箱上完整料號）的品項，領用時才能靠影像辨識認出來；其餘一律人工選。"
        style={{ marginBottom: 24 }}
      />

      <Card
        title="品項主檔"
        extra={unset > 0 ? <Text type="secondary">{unset} 項未設每箱數量（只能用箱數收貨）</Text> : null}
      >
        <Space style={{ marginBottom: 12 }} wrap>
          <Text type="secondary">
            這裡只有基本資料，不看庫存 —— 現在有多少去「庫存總覽」。
          直接在格子裡改，離開欄位就存；原物料名稱必填，其餘都可留空。欄位邊界可拖曳調寬。
          </Text>
          {itemCols.hasCustomWidths && (
            <Button size="small" type="link" onClick={itemCols.reset}>欄寬還原</Button>
          )}
        </Space>
        <Table
          rowKey="id"
          dataSource={items}
          pagination={false}
          size="middle"
          scroll={{ x: 1360 }}
          locale={{ emptyText: <Empty description="尚無品項" image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
          onRow={(row: Item) => rowDrag.rowProps(row)}
          columns={[
            ...(can("item.manage") ? [rowDrag.handleColumn()] : []),
            ...itemCols.resizable([
            {
              title: "原物料名稱", dataIndex: "name", width: 180,
              render: (v: string, row: Item) => (
                <TextCell value={v} strong onSave={(next) => patchField(row, "name", next)} />
              ),
            },
            {
              title: "型號", dataIndex: "model", width: 130,
              render: (v: string | null, row: Item) => (
                <TextCell value={v} placeholder="（無）"
                          onSave={(next) => patchField(row, "model", next)} />
              ),
            },
            {
              // Switchable, not derived: having a 型號 only means the label CAN
              // be matched — whether to actually use recognition for this item
              // is a judgement (small label, keeps misreading) that belongs to
              // whoever maintains the master.
              title: "影像辨識", width: 140, align: "center" as const,
              // No switch when there is nothing to match against. A disabled
              // toggle that still looks "on" next to the words 無可對映 says two
              // contradictory things at once; the honest rendering is to not
              // offer the choice and say what would enable it.
              render: (_, row: Item) =>
                row.matchable ? (
                  <Space size={4}>
                    <Switch
                      size="small"
                      checked={Boolean(row.use_recognition)}
                      onChange={(next) => patchField(row, "use_recognition", next)}
                    />
                    {row.use_recognition
                      ? <Tag color="blue">開啟</Tag>
                      : <Tag>關閉</Tag>}
                  </Space>
                ) : (
                  <Tooltip title="填了型號或箱上完整料號，才有東西可以跟標籤對映">
                    <Text type="secondary" style={{ fontSize: 12 }}>需型號或料號</Text>
                  </Tooltip>
                ),
            },
            {
              title: "規格", dataIndex: "spec", width: 140,
              render: (v: string | null, row: Item) => (
                <TextCell value={v} placeholder="—" onSave={(next) => patchField(row, "spec", next)} />
              ),
            },
            {
              title: "廠商", dataIndex: "supplier", width: 150,
              render: (v: string | null, row: Item) => (
                <CreatableSelect
                  value={v ?? undefined}
                  options={options.supplier}
                  placeholder="—"
                  addLabel="新增廠商"
                  category="supplier"
                  onAdded={load}
                  style={{ width: "100%" }}
                  onChange={(next) => patchField(row, "supplier", next)}
                />
              ),
            },
            {
              title: "箱上完整料號", dataIndex: "supplier_code", width: 220,
              render: (v: string | null, row: Item) => (
                <TextCell value={v} placeholder="未登記（辨識無法對映）"
                          onSave={(next) => patchField(row, "supplier_code", next)} />
              ),
            },
            {
              title: "每箱數量", dataIndex: "meters_per_box", width: 105, align: "right" as const,
              render: (v: number | null, row: Item) => (
                <NumberCell value={v} min={1} step={100} placeholder="未設"
                            onSave={(next) => patchField(row, "meters_per_box", next)} />
              ),
            },
            {
              // The unit belongs to the item: film is metres, foil bags are
              // sheets, desiccant is packs. A single global unit would be wrong
              // for most rows, so it travels with the item rather than the label.
              title: "單位", dataIndex: "pack_unit", width: 95,
              render: (v: string | null, row: Item) => (
                <CreatableSelect
                  value={v ?? undefined}
                  options={dictValues("pack_unit")}
                  placeholder="—"
                  addLabel="新增單位"
                  style={{ width: "100%" }}
                  onChange={(next) => patchField(row, "pack_unit", next)}
                />
              ),
            },
            {
              // Whether an item expires at all is a property of the item (the
              // form's note 1: 肉乾真空膜 needs no expiry date, 肉鬆 does). Saying
              // "yes" makes receiving insist on an actual 有效期限 — not a day
              // count to infer one from, because an inferred date is a date
              // nobody wrote down and nobody can check against the box.
              title: "有效期", dataIndex: "has_expiry", width: 130,
              render: (v: number, row: Item) => (
                <Space size={4}>
                  <Switch
                    size="small"
                    checked={Boolean(v)}
                    onChange={(next) => patchField(row, "has_expiry", next)}
                  />
                  <Tooltip title={v
                    ? "收貨時必須填有效期限"
                    : "收貨不會要求有效期限，也不發效期提醒"}>
                    <Text type="secondary" style={{ fontSize: 12 }}>{v ? "有" : "無"}</Text>
                  </Tooltip>
                </Space>
              ),
            },
            {
              // Kept because it is a setting, unlike on-hand which is a fact
              // about today and belongs on 庫存總覽.
              title: "安全水位", dataIndex: "safety_stock", width: 110,
              render: (v: number, row: Item) => (
                <NumberCell value={v} min={0} placeholder="未設"
                            onSave={(next) => patchField(row, "safety_stock", next ?? 0)} />
              ),
            },
            {
              title: "", width: 60, fixed: "right" as const, hidden: !can("item.manage"),
              render: (_, row: Item) =>
                row.on_hand > 0 || row.open_lots > 0 || row.rejected_qty > 0 ? (
                  <Tooltip title="已有進貨或領用紀錄，刪掉那些紀錄會失去對應">
                    <Button size="small" type="text" disabled>刪除</Button>
                  </Tooltip>
                ) : (
                  <Popconfirm
                    title={`刪除 ${row.label}？`}
                    okText="刪除" cancelText="取消" okButtonProps={{ danger: true }}
                    onConfirm={() => removeItem(row)}
                  >
                    <Button size="small" type="text" danger>刪除</Button>
                  </Popconfirm>
                ),
            },
          ])]}
          footer={can("item.manage") ? () => (
            <Form form={draftForm} layout="inline" onFinish={addItem} style={{ rowGap: 8 }}>
              <Form.Item
                name="name"
                rules={[{ required: true, message: "原物料名稱必填" }]}
                style={{ marginInlineEnd: 8 }}
              >
                <Input placeholder="原物料名稱（必填）" style={{ width: 190 }} />
              </Form.Item>
              <Form.Item name="model" style={{ marginInlineEnd: 8 }}>
                <Input placeholder="型號（可留空）" style={{ width: 150 }} />
              </Form.Item>
              <Form.Item name="spec" style={{ marginInlineEnd: 8 }}>
                <Input placeholder="規格" style={{ width: 140 }} />
              </Form.Item>
              <Form.Item name="supplier" style={{ marginInlineEnd: 8 }}>
                <CreatableSelect options={options.supplier} placeholder="廠商"
                                 addLabel="新增廠商" category="supplier" onAdded={load}
                                 style={{ width: 130 }} />
              </Form.Item>
              <Form.Item name="meters_per_box" style={{ marginInlineEnd: 8 }}>
                <InputNumber placeholder="每箱數量" min={1} step={100} style={{ width: 110 }} />
              </Form.Item>
              <Form.Item name="pack_unit" style={{ marginInlineEnd: 8 }}>
                <CreatableSelect options={dictValues("pack_unit")} placeholder="單位"
                                 addLabel="新增單位" style={{ width: 90 }} />
              </Form.Item>
              <Button type="primary" icon={<PlusOutlined />} htmlType="submit" loading={busy}>
                新增
              </Button>
            </Form>
          ) : undefined}
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
            { key: "items", label: `品項（${items.length}）`, children: itemsTab },
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
            // 人員與權限搬到「系統設定」了：誰能登入不是流程資料，是系統怎麼運作
          ]}
        />
      </Card>

    </>
  );
}
