"use client";

/**
 * Issuing screen.
 *
 * A draw is three facts: what was taken, and which lot — which is to say its
 * 進貨日 and 製造日. So that is the whole required path: pick the item, pick the
 * lot, submit. Machine, product and meter reading are columns off the paper
 * form; they matter for traceability but none of them should stand between an
 * operator and recording that a box left the shelf, so they are optional and
 * folded away.
 *
 * Recognition is an optional shortcut on top, not the way in —
 * requirement section 2.3 already said the manual path is the permanent one
 * ("辨識可斷，系統不可斷"), and until the recognition PoC has real numbers
 * behind it, making it the entrance would be betting the whole flow on it.
 *
 * Deviates from ChimesFlow density on purpose (declared mode-b override):
 * 64px touch targets and oversized type, because this is used with gloves on,
 * standing, possibly backlit. The verdict is a full colour block, not a toast.
 *
 * The copy on a blocked draw is "已記錄，請換一箱", never "失敗". A failure
 * message invites the operator to photograph the box again until it passes;
 * this wording says plainly that the record already exists and re-shooting
 * changes nothing (requirement section 2.1).
 */

import {
  CameraOutlined, InboxOutlined, ReloadOutlined, UnorderedListOutlined,
} from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Collapse, Empty, Form, Input, InputNumber, Radio,
  Row, Segmented, Select, Space, Spin, Tag, Tooltip, Typography, message,
} from "antd";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type Dictionary, type Item, type Lot, type Proposal } from "@/lib/api";

const { Title, Text } = Typography;

const TOUCH = 64;

type Verdict = { status: string; id: number; expected?: string } | null;

export default function IssuePage() {
  return (
    <Suspense fallback={<Card title="領用登錄" loading />}>
      <IssueScreen />
    </Suspense>
  );
}

function IssueScreen() {
  const params = useSearchParams();
  const [items, setItems] = useState<Item[]>([]);
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [itemId, setItemId] = useState<number>();
  const [lots, setLots] = useState<Lot[]>([]);
  const [lotId, setLotId] = useState<number>();
  // One box is the usual draw, so it is the default — but occasionally it is
  // more, and pretending otherwise would push people to record two draws or
  // just not record the second box.
  const [qty, setQty] = useState(1);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  // An explicit choice rather than a button in the corner: recognition and
  // manual entry are two ways of doing the same job, and which one is in play
  // changes what the screen is asking for. A small button made it look like an
  // extra, so nobody would reach for it — or would not know it existed.
  const [mode, setMode] = useState<"手動選擇" | "影像辨識">("手動選擇");
  const [capture, setCapture] = useState<Proposal | null>(null);
  const [verdict, setVerdict] = useState<Verdict>(null);
  const [form] = Form.useForm();
  const fileRef = useRef<HTMLInputElement>(null);

  const machines = (dict?.entries.machine ?? []).map((e) => e.value);
  const products = (dict?.entries.packed_product ?? []).map((e) => e.value);

  useEffect(() => {
    Promise.allSettled([api.items(), api.dictionary()]).then(([i, d]) => {
      if (i.status === "fulfilled") {
        setItems(i.value);
        const requested = params.get("item");
        const usable = (requested && i.value.find((r) => String(r.id) === requested))
          ?? i.value.find((r) => r.on_hand > 0);
        if (usable) setItemId(usable.id);
      } else {
        message.error(`品項載入失敗：${i.reason?.message}`);
      }
      if (d.status === "fulfilled") setDict(d.value);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadLots = useCallback(async (id: number) => {
    try {
      const rows = await api.lots(id);
      const drawable = rows.filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格");
      setLots(drawable);
      // Default to what FIFO wants. Pre-selecting the right answer means the
      // common case is one tap, and choosing otherwise is a deliberate act.
      const fifo = drawable.find((l) => l.is_fifo_next);
      setLotId(fifo?.id ?? drawable[0]?.id);
    } catch (e) {
      message.error((e as Error).message);
    }
  }, []);

  useEffect(() => {
    if (itemId) loadLots(itemId);
    else { setLots([]); setLotId(undefined); }
  }, [itemId, loadLots]);

  const selected = items.find((i) => i.id === itemId);
  const fifoLot = useMemo(() => lots.find((l) => l.is_fifo_next) ?? null, [lots]);
  const chosen = lots.find((l) => l.id === lotId);
  // Only warn about a genuinely later lot. A same-day lot is accepted by the
  // judgement, so warning about it would be crying wolf.
  const takingWrongLot = Boolean(chosen && !chosen.is_fifo_next && !chosen.fifo_also_ok);
  const chosenLot = lots.find((l) => l.id === lotId);
  useEffect(() => { setQty(1); }, [lotId]);

  async function onCapture(file: File) {
    setScanning(true);
    try {
      const result = await api.recognize(file);
      setCapture(result);
      if (result.item_id) {
        setItemId(result.item_id);
        const rows = await api.lots(result.item_id);
        const drawable = rows.filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格");
        setLots(drawable);
        setLotId(result.locked_lot?.lot_id ?? drawable.find((l) => l.is_fifo_next)?.id);
        message.success(
          result.locked_lot
            ? `辨識到 ${result.item_label}，進貨日 ${result.locked_lot.receipt_date}`
            : `辨識到 ${result.item_label}，但讀不出進貨日，請自己挑批次`,
        );
      } else {
        // Falling back rather than leaving them on a screen that cannot proceed:
        // requirement section 2.3 — recognition may fail, the system may not.
        setMode("手動選擇");
        message.warning("辨識不出是哪個品項，已切回手動選擇");
      }
    } catch (e) {
      setMode("手動選擇");
      message.error(`${(e as Error).message}　已切回手動選擇`);
    } finally {
      setScanning(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function submit() {
    if (!itemId || !lotId) return;
    const fields = await form.validateFields().catch(() => null);
    if (!fields) return;
    setBusy(true);
    try {
      const res = await api.createScan({
        item_id: itemId,
        lot_id: lotId,
        qty,
        image_path: capture?.image_path ?? null,
        ocr_receipt_date: capture?.recognition.receipt_date ?? null,
        ocr_confidence: capture?.recognition.confidence ?? null,
        ocr_notes: capture?.recognition.notes ?? null,
        match_distance: capture?.match_distance ?? null,
        fields,
        detail_pending: !fields["產品名稱"],
      });
      setVerdict({ status: res.status, id: res.id, expected: res.fifo_expected_date });
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  function reset() {
    setCapture(null);
    setVerdict(null);
    setQty(1);
    form.resetFields();
    if (itemId) loadLots(itemId);
  }

  if (verdict) return <VerdictBlock verdict={verdict} onNext={reset} />;

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>領用登錄</Title>

      <Card
        title="1. 選品項"
        extra={
          <Segmented
            value={mode}
            onChange={(v) => {
              setMode(v as "手動選擇" | "影像辨識");
              setCapture(null);
            }}
            options={[
              { value: "手動選擇", label: <Space size={4}><UnorderedListOutlined />手動選擇</Space> },
              { value: "影像辨識", label: <Space size={4}><CameraOutlined />影像辨識</Space> },
            ]}
          />
        }
      >
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          capture="environment"
          style={{ display: "none" }}
          onChange={(e) => e.target.files?.[0] && onCapture(e.target.files[0])}
        />

        {mode === "影像辨識" ? (
          <>
            <Button
              type="primary"
              size="large"
              icon={<CameraOutlined />}
              loading={scanning}
              onClick={() => fileRef.current?.click()}
              style={{ height: TOUCH, fontSize: 20, width: "100%" }}
            >
              {capture ? "重拍" : "拍照或選擇照片"}
            </Button>
            <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
              對準側面的標籤和紅色驗收章。認得出就自動帶入品項與批次；
              認不出會切回手動，不會卡住你。
            </Text>
          </>
        ) : (
          <>
            <Select
              size="large"
              style={{ width: "100%", height: TOUCH }}
              value={itemId}
              onChange={(v) => { setItemId(v); setCapture(null); }}
              showSearch
              optionFilterProp="label"
              placeholder="選要領用的品項"
              options={items.map((i) => ({
                value: i.id,
                label: `${i.label}｜${i.name}${i.spec ? ` ${i.spec}` : ""}（在庫 ${i.on_hand} 箱）`,
                disabled: i.on_hand === 0,
              }))}
            />
            <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
              在庫 0 的品項不能選。
            </Text>
          </>
        )}

        {capture && (
          <Alert
            style={{ marginTop: 16 }}
            type={capture.item_id ? "info" : "warning"}
            title={capture.item_id ? `已由照片帶入：${capture.item_label}` : "照片認不出品項，已切回手動"}
            description={
              <Space orientation="vertical" size={2}>
                <span>
                  標籤讀到型號：{capture.recognition.model_code
                    ?? capture.recognition.item_code ?? "讀不出"}
                  ｜章：{capture.recognition.receipt_date ?? "讀不出"}
                </span>
                {capture.recognition.notes && (
                  <Text type="secondary">{capture.recognition.notes}</Text>
                )}
              </Space>
            }
            action={<Button size="small" icon={<ReloadOutlined />} onClick={reset}>清除</Button>}
          />
        )}

        {mode === "影像辨識" && itemId && (
          <div style={{ marginTop: 16 }}>
            {/* Only claim recognition when a photo actually produced this. An
                item carried over from a manual pick is not something the camera
                found, and saying so would make the screen untrustworthy on the
                one thing it most needs to be honest about. */}
            <Text type="secondary">
              {capture?.item_id === itemId ? "照片辨識到：" : "目前選定（手動）："}
            </Text>{" "}
            <Text strong style={{ fontSize: 18 }}>{selected?.label}</Text>{" "}
            <Text>{selected?.name}</Text>{" "}
            <Button size="small" type="link" onClick={() => setMode("手動選擇")}>換一個</Button>
          </div>
        )}
      </Card>

      {itemId && lots.length === 0 && (
        <Card title="還不能領用" style={{ marginTop: 24 }}>
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={`${selected?.label ?? "此品項"} 沒有可領用的在庫批次`}
          >
            <Link href="/receiving">
              <Button type="primary" icon={<InboxOutlined />}>去收貨建批</Button>
            </Link>
          </Empty>
        </Card>
      )}

      {lots.length > 0 && (
        <>
          <Card title="2. 選批次（進貨日／製造日）" style={{ marginTop: 24 }}>
            <Radio.Group
              value={lotId}
              onChange={(e) => setLotId(e.target.value)}
              style={{ display: "block" }}
            >
              <Space orientation="vertical" style={{ width: "100%" }} size={12}>
                {lots.map((lot) => (
                  <Radio
                    key={lot.id}
                    value={lot.id}
                    style={{
                      width: "100%", minHeight: TOUCH, padding: "12px 16px",
                      border: `1px solid ${lot.id === lotId ? "#1677ff" : "#d9d9d9"}`,
                      borderRadius: 6, fontSize: 18,
                      background: lot.id === lotId ? "#e6f4ff" : "#fff",
                    }}
                  >
                    <Space wrap>
                      <span style={{ fontSize: 18 }}>進貨 {lot.receipt_date}</span>
                      <span style={{ fontSize: 18 }}>製造 {lot.manufacture_date ?? "—"}</span>
                      {lot.effective_expiry && (
                        <Text type={lot.days_left != null && lot.days_left <= 90 ? "danger" : "secondary"}>
                          有效 {lot.effective_expiry}
                          {lot.days_left != null && `（剩 ${lot.days_left} 天）`}
                        </Text>
                      )}
                      <Text type="secondary">在庫 {lot.qty_on_hand} 箱</Text>
                      {lot.is_fifo_next && <Tag color="green">FIFO 應領</Tag>}
                      {lot.fifo_also_ok && (
                        <Tooltip title="跟應領那批同一個進貨日，領這批也合法，不會被擋">
                          <Tag>同進貨日．可領</Tag>
                        </Tooltip>
                      )}
                    </Space>
                  </Radio>
                ))}
              </Space>
            </Radio.Group>

            {takingWrongLot && (
              <Alert
                style={{ marginTop: 16 }}
                type="warning"
                title="這不是 FIFO 應領的那批"
                description={`應領用進貨日 ${fifoLot?.receipt_date} 那批。仍然可以送出 —— 系統會記錄下來並擋住扣帳，等主管覆核。`}
              />
            )}
          </Card>

          <Card title="3. 數量" style={{ marginTop: 24 }}>
            <Space align="center" size={16} wrap>
              <Button
                size="large" style={{ height: TOUCH, width: TOUCH, fontSize: 28 }}
                disabled={qty <= 1}
                onClick={() => setQty((n) => Math.max(1, n - 1))}
              >
                −
              </Button>
              <InputNumber
                size="large"
                min={1}
                max={chosenLot?.qty_on_hand ?? 1}
                value={qty}
                onChange={(v) => setQty(Math.max(1, Number(v) || 1))}
                style={{ width: 120, height: TOUCH, fontSize: 24 }}
              />
              <Button
                size="large" style={{ height: TOUCH, width: TOUCH, fontSize: 28 }}
                disabled={qty >= (chosenLot?.qty_on_hand ?? 1)}
                onClick={() => setQty((n) => n + 1)}
              >
                ＋
              </Button>
              <Text style={{ fontSize: 18 }}>箱</Text>
              <Text type="secondary">
                這批在庫 {chosenLot?.qty_on_hand ?? 0} 箱
              </Text>
            </Space>
          </Card>

          <Card style={{ marginTop: 24 }}>
            <Form form={form} layout="vertical" size="large">
              <Collapse
                ghost
                items={[{
                  key: "extra",
                  label: <Text type="secondary">其他資訊（選填，可事後補）</Text>,
                  children: (
                    <Row gutter={24}>
                      <Col xs={24} md={8}>
                        <Form.Item name="包裝機台" label="包裝機台">
                          <Select
                            style={{ height: TOUCH }}
                            allowClear
                            placeholder="選機台"
                            options={machines.map((m) => ({ value: m, label: m }))}
                          />
                        </Form.Item>
                      </Col>
                      <Col xs={24} md={8}>
                        <Form.Item name="產品名稱" label="產品名稱">
                          <Select
                            style={{ height: TOUCH }}
                            allowClear
                            placeholder="包了哪個產品"
                            options={products.map((p) => ({ value: p, label: p }))}
                          />
                        </Form.Item>
                      </Col>
                      <Col xs={24} md={8}>
                        <Form.Item name="使用步徑" label="使用步徑（碼表）">
                          <Input style={{ height: TOUCH, fontSize: 18 }} placeholder="例 557" />
                        </Form.Item>
                      </Col>
                    </Row>
                  ),
                }]}
              />
              <Button
                type="primary"
                size="large"
                loading={busy}
                disabled={!lotId}
                onClick={submit}
                style={{ height: TOUCH, fontSize: 20, width: "100%", marginTop: 8 }}
              >
                送出領用
              </Button>
              <Text type="secondary" style={{ display: "block", marginTop: 12, textAlign: "center" }}>
                {selected && chosenLot
                  ? `領用 ${selected.label} ${qty} 箱｜進貨 ${chosenLot.receipt_date}｜製造 ${chosenLot.manufacture_date ?? "—"}`
                  : "選好品項與批次就可以送出"}
              </Text>
            </Form>
          </Card>
        </>
      )}

      {scanning && !capture && (
        <Card style={{ marginTop: 24, textAlign: "center" }} title="辨識中">
          <Spin size="large" />
        </Card>
      )}
    </>
  );
}

function VerdictBlock({ verdict, onNext }: { verdict: NonNullable<Verdict>; onNext: () => void }) {
  const blocked = verdict.status === "blocked_fifo";
  const unreadable = verdict.status === "blocked_unreadable";
  const background = blocked ? "#ff4d4f" : unreadable ? "#faad14" : "#52c41a";
  const heading = blocked ? "已記錄，請換一箱" : unreadable ? "已記錄，待人工補批次" : "已登錄";

  return (
    <div
      style={{
        background, borderRadius: 6, color: "#fff", padding: 48,
        minHeight: "60vh", display: "flex", flexDirection: "column", justifyContent: "center",
      }}
    >
      <Title level={1} style={{ color: "#fff", margin: 0, fontSize: 48 }}>{heading}</Title>
      {blocked && (
        <Title level={3} style={{ color: "#fff", marginTop: 16, fontWeight: 400 }}>
          應領用：進貨日 {verdict.expected} 那批
        </Title>
      )}
      <Text style={{ color: "#fff", fontSize: 18, marginTop: 24, opacity: 0.9 }}>
        {blocked
          ? `這一筆已存成紀錄 #${verdict.id}，包含時間與登記人。重拍或重送不會讓它消失，請去拿較早進貨的那箱。`
          : `紀錄 #${verdict.id} 已建立。`}
      </Text>
      <Button
        size="large"
        onClick={onNext}
        style={{ height: TOUCH, fontSize: 20, marginTop: 40, alignSelf: "flex-start", minWidth: 200 }}
      >
        下一箱
      </Button>
    </div>
  );
}
