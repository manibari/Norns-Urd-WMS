"use client";

/**
 * 領用登錄 — photo first.
 *
 * The operator walks up with a box already in their hands. They know what it
 * is; the system is the one that needs to find out. So the screen opens on a
 * camera, not a dropdown, and the first thing it says back is a verdict:
 * 可以拿 / 先別拿. Asking "which item are you drawing?" before the photo made
 * the person do the identification and left the camera to confirm what they
 * had already typed — backwards, and it quietly narrowed the candidate set by
 * hand, which is the one thing §2.2 says the machine must not be helped with.
 *
 * The manual path is still permanently reachable (§2.3 辨識可斷，系統不可斷) —
 * one tap away, and entered automatically whenever recognition cannot resolve
 * the box. It is a fallback now, not the front door.
 *
 * Copy on a blocked draw is "已記錄，請換一箱", never "失敗". A failure message
 * invites re-shooting the box until it passes; this wording says the record
 * already exists and re-shooting changes nothing (§2.1).
 *
 * Deviates from ChimesFlow density on purpose (declared mode-b override):
 * 64px touch targets and oversized type, because this is used with gloves on,
 * standing, possibly backlit. The verdict is a full colour block, not a toast.
 */

import {
  CameraOutlined, CheckCircleFilled, CloseCircleFilled, InboxOutlined,
  QuestionCircleFilled, ReloadOutlined, UnorderedListOutlined, VideoCameraOutlined,
} from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Collapse, Empty, Form, Input, InputNumber, Radio,
  Row, Select, Space, Spin, Tag, Tooltip, Typography, message,
} from "antd";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type Dictionary, type Item, type Lot, type Proposal } from "@/lib/api";

const { Title, Text } = Typography;

const TOUCH = 64;
const OK = "#52c41a";
const BAD = "#ff4d4f";
const WARN = "#faad14";

type Verdict = { status: string; id: number; expected?: string } | null;
/** 拍照 → 判定；認不出來才進手動 */
type Stage = "拍照" | "判定" | "手動";

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
  const [cameraOn, setCameraOn] = useState(false);
  const [stage, setStage] = useState<Stage>("拍照");
  const [itemId, setItemId] = useState<number>();
  const [lots, setLots] = useState<Lot[]>([]);
  const [lotId, setLotId] = useState<number>();
  // One box is the usual draw, so it is the default — but occasionally it is
  // more, and pretending otherwise would push people to record two draws or
  // just not record the second box.
  const [qty, setQty] = useState(1);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [capture, setCapture] = useState<Proposal | null>(null);
  const [verdict, setVerdict] = useState<Verdict>(null);
  const [form] = Form.useForm();
  const fileRef = useRef<HTMLInputElement>(null);

  const machines = (dict?.entries.machine ?? []).map((e) => e.value);
  const products = (dict?.entries.packed_product ?? []).map((e) => e.value);

  useEffect(() => {
    Promise.allSettled([api.items(), api.dictionary(), api.camera()]).then(([i, d, c]) => {
      if (i.status === "fulfilled") setItems(i.value);
      else message.error(`品項載入失敗：${i.reason?.message}`);
      if (d.status === "fulfilled") setDict(d.value);
      if (c.status === "fulfilled") setCameraOn(c.value.enabled && Boolean(c.value.endpoint));
      // Arriving from 庫存總覽's 領用 link names the item, which is a deliberate
      // manual entry — honour it rather than demanding a photo.
      const requested = params.get("item");
      if (requested && i.status === "fulfilled" && i.value.some((r) => String(r.id) === requested)) {
        setItemId(Number(requested));
        setStage("手動");
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadLots = useCallback(async (id: number, prefer?: number) => {
    try {
      const rows = await api.lots(id);
      const drawable = rows.filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格");
      setLots(drawable);
      // Default to what FIFO wants. Pre-selecting the right answer means the
      // common case is one tap, and choosing otherwise is a deliberate act.
      setLotId(prefer ?? drawable.find((l) => l.is_fifo_next)?.id ?? drawable[0]?.id);
      return drawable;
    } catch (e) {
      message.error((e as Error).message);
      return [];
    }
  }, []);

  useEffect(() => {
    if (stage === "手動" && itemId) loadLots(itemId);
    else if (!itemId) { setLots([]); setLotId(undefined); }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemId, stage]);

  const selected = items.find((i) => i.id === itemId);
  const fifoLot = useMemo(() => lots.find((l) => l.is_fifo_next) ?? null, [lots]);
  const chosenLot = lots.find((l) => l.id === lotId);
  // Only warn about a genuinely later lot. A same-day lot is accepted by the
  // judgement, so warning about it would be crying wolf.
  const takingWrongLot = Boolean(chosenLot && !chosenLot.is_fifo_next && !chosenLot.fifo_also_ok);

  async function runScan(source: () => Promise<Proposal>) {
    setScanning(true);
    try {
      const result = await source();
      setCapture(result);
      if (!result.item_id) {
        // Recognition may fail, the system may not (§2.3). Drop into manual with
        // the reason on screen rather than leaving them somewhere unusable.
        setStage("手動");
        setItemId(undefined);
        return;
      }
      setItemId(result.item_id);
      await loadLots(result.item_id, result.locked_lot?.lot_id);
      setStage("判定");
    } catch (e) {
      setStage("手動");
      message.error(`${(e as Error).message}　已切到手動登錄`);
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

  function reset(to: Stage = "拍照") {
    setCapture(null);
    setVerdict(null);
    setQty(1);
    form.resetFields();
    setStage(to);
    if (to === "拍照") { setItemId(undefined); setLots([]); setLotId(undefined); }
  }

  if (verdict) return <VerdictBlock verdict={verdict} onNext={() => reset("拍照")} />;

  const scanButtons = (
    <>
      <input
        ref={fileRef}
        type="file"
        accept="image/*"
        capture="environment"
        style={{ display: "none" }}
        onChange={(e) => e.target.files?.[0] && runScan(() => api.recognize(e.target.files![0]))}
      />
      <Space orientation="vertical" size={12} style={{ width: "100%" }}>
        <Button
          type="primary"
          size="large"
          icon={<CameraOutlined />}
          loading={scanning}
          onClick={() => fileRef.current?.click()}
          style={{ height: 96, fontSize: 26, width: "100%" }}
        >
          拍這一箱
        </Button>
        {cameraOn && (
          <Button
            size="large"
            icon={<VideoCameraOutlined />}
            loading={scanning}
            onClick={() => runScan(api.captureFromCamera)}
            style={{ height: TOUCH, fontSize: 18, width: "100%" }}
          >
            用固定相機拍
          </Button>
        )}
      </Space>
    </>
  );

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>領用登錄</Title>

      {stage === "拍照" && (
        <Card>
          {scanning ? (
            <div style={{ textAlign: "center", padding: "48px 0" }}>
              <Spin size="large" />
              <div style={{ fontSize: 20, marginTop: 20 }}>辨識中，看是哪一箱…</div>
            </div>
          ) : (
            <>
              <div style={{ fontSize: 22, fontWeight: 600, marginBottom: 8 }}>
                把箱子放好，拍側面的標籤和紅色驗收章
              </div>
              <Text type="secondary" style={{ fontSize: 16, display: "block", marginBottom: 24 }}>
                不用先選要領什麼 —— 拍完系統會告訴你這箱能不能拿出去。
              </Text>
              {scanButtons}
              <div style={{ marginTop: 24, textAlign: "center" }}>
                <Button
                  type="link"
                  icon={<UnorderedListOutlined />}
                  onClick={() => setStage("手動")}
                  style={{ fontSize: 16 }}
                >
                  相機壞了 / 這箱沒有標籤 → 手動登錄
                </Button>
              </div>
            </>
          )}
        </Card>
      )}

      {stage === "判定" && capture && (
        <ScanVerdict
          capture={capture}
          lots={lots}
          onRetake={() => reset("拍照")}
          onManual={() => setStage("手動")}
        />
      )}

      {stage === "手動" && (
        <Card
          title="選品項"
          extra={
            <Button icon={<CameraOutlined />} onClick={() => reset("拍照")}>改用拍照</Button>
          }
        >
          {capture && !capture.item_id && (
            <Alert
              style={{ marginBottom: 16 }}
              type="warning"
              title="這張照片認不出是哪個品項，請自己選"
              description={
                <Space orientation="vertical" size={2}>
                  <span>
                    標籤讀到：{capture.recognition.model_code ?? capture.recognition.item_code ?? "讀不出型號"}
                    ｜章：{capture.recognition.receipt_date ?? "讀不出"}
                  </span>
                  {capture.recognition.notes && (
                    <Text type="secondary">{capture.recognition.notes}</Text>
                  )}
                  <Text type="secondary">照片已留存，會跟著這筆紀錄一起存檔。</Text>
                </Space>
              }
              action={<Button size="small" icon={<ReloadOutlined />} onClick={() => reset("拍照")}>重拍</Button>}
            />
          )}
          <Select
            size="large"
            style={{ width: "100%", height: TOUCH }}
            value={itemId}
            onChange={setItemId}
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
        </Card>
      )}

      {stage !== "拍照" && itemId && lots.length === 0 && (
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

      {stage !== "拍照" && lots.length > 0 && (
        <>
          {/* On the recognised path the lot is already decided, so the picker
              only appears when a human has to make the call — either they came
              in manually, or the stamp could not be read. */}
          {(stage === "手動" || !capture?.locked_lot) && (
            <Card title="選批次（先進先出看製造日）" style={{ marginTop: 24 }}>
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
                        {/* Manufacture date leads: it is what FIFO sorts on, so
                            it is what someone should be reading off the box. */}
                        <span style={{ fontSize: 18, fontWeight: 600 }}>
                          製造 {lot.manufacture_date ?? "未填"}
                        </span>
                        {lot.is_fifo_next && <Tag color="green">FIFO 應領</Tag>}
                        {lot.fifo_also_ok && (
                          <Tooltip title="跟應領那批同一個製造日，領這批也合法，不會被擋">
                            <Tag>同製造日．可領</Tag>
                          </Tooltip>
                        )}
                        <Text type="secondary">進貨 {lot.receipt_date}</Text>
                        {lot.effective_expiry && (
                          <Text type={lot.days_left != null && lot.days_left <= 90 ? "danger" : "secondary"}>
                            有效 {lot.effective_expiry}
                            {lot.days_left != null && `（剩 ${lot.days_left} 天）`}
                          </Text>
                        )}
                        <Text type="secondary">在庫 {lot.qty_on_hand} 箱</Text>
                      </Space>
                    </Radio>
                  ))}
                </Space>
              </Radio.Group>

              {lots[0].fifo_basis === "進貨日期" && (
                <Alert
                  style={{ marginTop: 16 }}
                  type="warning"
                  title="這個品項的批次都沒填製造日期，暫時改用進貨日排序"
                  description="先進先出看的是製造日期。沒有製造日就只能退而求其次用進貨日 —— 補上製造日期後排序才會準。"
                />
              )}

              {takingWrongLot && (
                <Alert
                  style={{ marginTop: 16 }}
                  type="warning"
                  title="這不是 FIFO 應領的那批"
                  description={
                    `應領用製造日 ${fifoLot?.manufacture_date ?? fifoLot?.receipt_date} 那批`
                    + `（進貨 ${fifoLot?.receipt_date}）。仍然可以送出 —— 系統會記錄下來並擋住扣帳，等主管覆核。`
                  }
                />
              )}
            </Card>
          )}

          <Card style={{ marginTop: 24 }}>
            <Space align="center" size={16} wrap style={{ marginBottom: 20 }}>
              <Text style={{ fontSize: 18 }}>領幾箱</Text>
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
              <Text type="secondary">這批在庫 {chosenLot?.qty_on_hand ?? 0} 箱</Text>
            </Space>

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
                danger={takingWrongLot}
                onClick={submit}
                style={{ height: TOUCH, fontSize: 20, width: "100%", marginTop: 8 }}
              >
                {takingWrongLot ? `還是要領這箱（會記錄、不扣帳）` : `確認領用 ${qty} 箱`}
              </Button>
              <Text type="secondary" style={{ display: "block", marginTop: 12, textAlign: "center" }}>
                {selected && chosenLot
                  ? `${selected.label}｜${qty} 箱｜製造 ${chosenLot.manufacture_date ?? "未填"}｜進貨 ${chosenLot.receipt_date}`
                  : "選好品項與批次就可以送出"}
              </Text>
            </Form>
          </Card>
        </>
      )}
    </>
  );
}

/**
 * The answer to the only question the operator asked: can this box go out?
 *
 * Colour and heading carry the verdict on their own, because it gets read from
 * a few steps away before anyone leans in for the detail.
 */
function ScanVerdict({ capture, lots, onRetake, onManual }: {
  capture: Proposal; lots: Lot[]; onRetake: () => void; onManual: () => void;
}) {
  const locked = capture.locked_lot;
  const fifoLot = lots.find((l) => l.is_fifo_next) ?? null;

  // Three states, and they are genuinely different: this box is fine / this box
  // is the wrong one / we could not tell which box this is. Collapsing the last
  // two into "failed" would tell someone to go swap a box we never identified.
  const state = !locked ? "unknown" : capture.fifo_ok ? "ok" : "wrong";
  const colour = state === "ok" ? OK : state === "wrong" ? BAD : WARN;
  const heading = state === "ok" ? "可以拿" : state === "wrong" ? "這箱先別拿" : "認得出品項，但讀不出是哪一批";
  const Icon = state === "ok" ? CheckCircleFilled : state === "wrong" ? CloseCircleFilled : QuestionCircleFilled;

  return (
    <>
      <div style={{ background: colour, borderRadius: 8, color: "#fff", padding: 32 }}>
        <Space size={16} align="start">
          <Icon style={{ fontSize: 52 }} />
          <div>
            <div style={{ fontSize: 44, fontWeight: 700, lineHeight: 1.15 }}>{heading}</div>
            <div style={{ fontSize: 24, marginTop: 10 }}>
              {capture.item_name}
              {capture.item_model && (
                <span style={{ opacity: 0.85, fontSize: 20 }}>　{capture.item_model}</span>
              )}
            </div>
          </div>
        </Space>

        {state === "ok" && locked && (
          <div style={{ fontSize: 20, marginTop: 20, lineHeight: 1.7 }}>
            製造 {locked.manufacture_date ?? "未填"}
            ｜進貨 {locked.receipt_date}
            ｜這批還有 {locked.qty_on_hand} 箱
            <div style={{ opacity: 0.9, fontSize: 17, marginTop: 4 }}>
              這就是先進先出該領的那一批。
            </div>
          </div>
        )}

        {state === "wrong" && locked && (
          <div style={{ fontSize: 20, marginTop: 20, lineHeight: 1.7 }}>
            <div>你這箱：製造 {locked.manufacture_date ?? "未填"}（進貨 {locked.receipt_date}）</div>
            <div style={{ fontWeight: 700 }}>
              該拿的是：製造 {capture.fifo_expected_date} 那批
              {fifoLot && `（進貨 ${fifoLot.receipt_date}，還有 ${fifoLot.qty_on_hand} 箱）`}
            </div>
          </div>
        )}

        {state === "unknown" && (
          <div style={{ fontSize: 19, marginTop: 20, lineHeight: 1.7 }}>
            章讀到「{capture.recognition.receipt_date ?? "讀不出"}」，跟在庫的批次都對不起來。
            <div style={{ opacity: 0.9, fontSize: 17 }}>照片已留存。請在下面挑正確的批次。</div>
          </div>
        )}
      </div>

      <Space style={{ marginTop: 16 }} wrap>
        <Button icon={<ReloadOutlined />} onClick={onRetake} style={{ height: 48, fontSize: 16 }}>
          {state === "wrong" ? "去換一箱，重拍" : "重拍"}
        </Button>
        <Button type="link" onClick={onManual} style={{ fontSize: 16 }}>辨識錯了？改手動選</Button>
        {capture.image_path && (
          <a href={capture.image_path} target="_blank" rel="noreferrer" style={{ fontSize: 16 }}>看照片</a>
        )}
      </Space>

      {state === "wrong" && (
        <Alert
          style={{ marginTop: 16 }}
          type="warning"
          title="還是要領這箱的話，往下送出"
          description="系統會完整記錄下來（誰、何時、拿了哪批、應拿哪批、照片），但不扣帳，等主管覆核。重拍不會讓這筆紀錄消失。"
        />
      )}
    </>
  );
}

function VerdictBlock({ verdict, onNext }: { verdict: NonNullable<Verdict>; onNext: () => void }) {
  const blocked = verdict.status === "blocked_fifo";
  const unreadable = verdict.status === "blocked_unreadable";
  const background = blocked ? BAD : unreadable ? WARN : OK;
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
          應領用：製造日 {verdict.expected} 那批
        </Title>
      )}
      <Text style={{ color: "#fff", fontSize: 18, marginTop: 24, opacity: 0.9 }}>
        {blocked
          ? `這一筆已存成紀錄 #${verdict.id}，包含時間與登記人。重拍或重送不會讓它消失，請去拿製造日較早的那箱。`
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
