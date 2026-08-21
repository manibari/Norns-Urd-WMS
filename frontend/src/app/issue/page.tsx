"use client";

/**
 * Issuing screen — the one operators use on the packing line.
 *
 * Deviates from ChimesFlow density on purpose (declared mode-b override):
 * 64px touch targets and oversized type, because this is used with gloves on,
 * standing, possibly backlit. The verdict is a full colour block rather than a
 * toast for the same reason.
 *
 * The copy on a blocked draw is "已記錄，請換一箱", never "失敗". A failure
 * message invites the operator to photograph the box again until it passes;
 * this wording says plainly that the record already exists and re-shooting
 * changes nothing (requirement section 2.1).
 */

import { CameraOutlined, InboxOutlined, ReloadOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Descriptions, Empty, Form, Input, Radio,
  Row, Select, Space, Spin, Tag, Typography, message,
} from "antd";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo, useRef, useState } from "react";
import { api, type Item, type Proposal } from "@/lib/api";

const { Title, Text } = Typography;

const TOUCH = 64;
const MACHINES = ["D-003", "D-004", "D-023", "D-027"];
const PRODUCTS = ["經典", "豬切", "牛切", "休閒豬", "金尊", "海香", "魚絲", "雞脆"];

type Verdict = { status: string; id: number; expected?: string } | null;

export default function IssuePage() {
  // Wrapped because useSearchParams opts the tree into client rendering.
  return (
    <Suspense fallback={<Card title="領用登錄" loading />}>
      <IssueScreen />
    </Suspense>
  );
}

function IssueScreen() {
  const params = useSearchParams();
  const [items, setItems] = useState<Item[]>([]);
  const [itemCode, setItemCode] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [chosenLot, setChosenLot] = useState<number>();
  const [verdict, setVerdict] = useState<Verdict>(null);
  const [form] = Form.useForm();
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.items().then((rows) => {
      setItems(rows);
      // Default to something that can actually be drawn. Landing on an item with
      // no stock looks like the system is broken rather than like a prerequisite
      // is missing.
      // An item named in the URL wins — that link came from receiving, where
      // someone just put this exact lot on the shelf.
      const requested = params.get("item");
      const usable = (requested && rows.find((r) => r.item_code === requested))
        ?? rows.find((r) => r.on_hand > 0)
        ?? rows[0];
      if (usable && !itemCode) setItemCode(usable.item_code);
    }).catch((e) => message.error(e.message));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const selected = items.find((i) => i.item_code === itemCode);
  const noStock = Boolean(selected && selected.on_hand === 0);

  const fifoTarget = useMemo(() => {
    if (!proposal?.candidates.length) return null;
    return proposal.candidates.reduce((a, b) => (a.receipt_date <= b.receipt_date ? a : b));
  }, [proposal]);

  async function onCapture(file: File) {
    if (!itemCode) return;
    setBusy(true);
    setVerdict(null);
    try {
      const result = await api.recognize(itemCode, file);
      setProposal(result);
      setChosenLot(result.locked_lot?.lot_id);
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function submit() {
    if (!proposal || !itemCode) return;
    const fields = await form.validateFields().catch(() => null);
    if (!fields) return;
    setBusy(true);
    try {
      const res = await api.createScan({
        item_code: itemCode,
        lot_id: chosenLot ?? null,
        image_path: proposal.image_path,
        ocr_receipt_date: proposal.recognition.receipt_date,
        ocr_confidence: proposal.recognition.confidence,
        ocr_notes: proposal.recognition.notes,
        match_distance: proposal.match_distance,
        fields,
        detail_pending: !fields["包裝產品"],
      });
      setVerdict({ status: res.status, id: res.id, expected: res.fifo_expected_date });
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  function reset() {
    setProposal(null);
    setChosenLot(undefined);
    setVerdict(null);
    form.resetFields();
  }

  if (verdict) return <VerdictBlock verdict={verdict} onNext={reset} />;

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>領用登錄</Title>

      <Card title="1. 選料號">
        <Select
          size="large"
          style={{ width: "100%", height: TOUCH }}
          value={itemCode}
          onChange={(v) => { setItemCode(v); reset(); }}
          options={items.map((i) => ({
            value: i.item_code,
            label: `${i.name}｜${i.item_code}（在庫 ${i.on_hand} ${i.unit}／${i.open_lots} 批）`,
          }))}
        />
      </Card>

      {(items.length === 0 || noStock) && (
        <Card style={{ marginTop: 24 }} title="還不能領用">
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              items.length === 0
                ? "尚無任何品項，請先去收貨建批"
                : `${selected?.name} 目前沒有在庫批次，請先收貨建批`
            }
          >
            <Link href="/">
              <Button type="primary" size="large" icon={<InboxOutlined />} style={{ height: TOUCH, fontSize: 18 }}>
                去收貨建批
              </Button>
            </Link>
          </Empty>
          <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
            FIFO 要排序就得知道這個料號還有哪幾批在庫。沒有批次資料，拍照也比對不出東西。
          </Text>
        </Card>
      )}

      {!noStock && items.length > 0 && (
      <Card title="2. 拍箱子" style={{ marginTop: 24 }}>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          capture="environment"
          style={{ display: "none" }}
          onChange={(e) => e.target.files?.[0] && onCapture(e.target.files[0])}
        />
        <Button
          type="primary"
          size="large"
          icon={<CameraOutlined />}
          loading={busy}
          disabled={!itemCode}
          onClick={() => fileRef.current?.click()}
          style={{ height: TOUCH, fontSize: 20, width: "100%" }}
        >
          拍照或選擇照片
        </Button>
        <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
          對準側面的紅色驗收章。讀不到不會擋你，會讓你自己挑批次。
        </Text>
      </Card>
      )}

      {busy && !proposal && (
        <Card style={{ marginTop: 24, textAlign: "center" }} title="辨識中">
          <Spin size="large" />
        </Card>
      )}

      {proposal && (
        <>
          <Card
            title="3. 確認批次"
            style={{ marginTop: 24 }}
            extra={<Button icon={<ReloadOutlined />} onClick={reset}>重拍</Button>}
          >
            <Row gutter={24}>
              <Col xs={24} md={10}>
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img
                  src={proposal.image_path}
                  alt="領用箱影像"
                  style={{ width: "100%", borderRadius: 6, border: "1px solid #f0f0f0" }}
                />
              </Col>
              <Col xs={24} md={14}>
                {proposal.decision === "lock" ? (
                  <Alert
                    type="info"
                    title={`辨識到進貨日 ${proposal.recognition.receipt_date}`}
                    description={`信心 ${(proposal.recognition.confidence * 100).toFixed(0)}%，比對到在庫批次。請確認這是你手上那一箱。`}
                    style={{ marginBottom: 16 }}
                  />
                ) : (
                  <Alert
                    type="warning"
                    title="讀不出進貨日，請自己挑批次"
                    description={
                      proposal.recognition.error
                        ? `辨識服務異常：${proposal.recognition.error}`
                        : proposal.recognition.notes || "章可能糊掉、被遮住，或這面沒有章。"
                    }
                    style={{ marginBottom: 16 }}
                  />
                )}

                <Text strong style={{ fontSize: 16 }}>在庫批次（請點選你手上這箱）</Text>
                <Radio.Group
                  value={chosenLot}
                  onChange={(e) => setChosenLot(e.target.value)}
                  style={{ display: "block", marginTop: 12 }}
                >
                  <Space direction="vertical" style={{ width: "100%" }} size={12}>
                    {proposal.candidates.map((c) => (
                      <Radio
                        key={c.lot_id}
                        value={c.lot_id}
                        style={{
                          width: "100%", minHeight: TOUCH, padding: "12px 16px",
                          border: "1px solid #d9d9d9", borderRadius: 6, fontSize: 18,
                          background: c.lot_id === chosenLot ? "#e6f4ff" : "#fff",
                        }}
                      >
                        <Space>
                          <span style={{ fontSize: 18 }}>進貨 {c.receipt_date}</span>
                          <Text type="secondary">製造 {c.manufacture_date ?? "—"}</Text>
                          <Text type="secondary">在庫 {c.qty_on_hand} 箱</Text>
                          {fifoTarget?.lot_id === c.lot_id && <Tag color="green">FIFO 應領</Tag>}
                        </Space>
                      </Radio>
                    ))}
                  </Space>
                </Radio.Group>
                {!proposal.candidates.length && (
                  <Empty description="此料號尚無在庫批次" image={Empty.PRESENTED_IMAGE_SIMPLE} />
                )}
              </Col>
            </Row>
          </Card>

          <Card title="4. 填領用資訊" style={{ marginTop: 24 }}>
            <Form form={form} layout="vertical" size="large" initialValues={{ 上下膜: "上膜" }}>
              <Row gutter={24}>
                <Col xs={24} md={6}>
                  <Form.Item name="上下膜" label="上膜／下膜" rules={[{ required: true }]}>
                    <Radio.Group buttonStyle="solid" size="large">
                      <Radio.Button value="上膜" style={{ height: TOUCH, lineHeight: `${TOUCH}px`, fontSize: 18 }}>上膜</Radio.Button>
                      <Radio.Button value="下膜" style={{ height: TOUCH, lineHeight: `${TOUCH}px`, fontSize: 18 }}>下膜</Radio.Button>
                    </Radio.Group>
                  </Form.Item>
                </Col>
                <Col xs={24} md={6}>
                  <Form.Item name="包裝機台" label="包裝機台" rules={[{ required: true }]}>
                    <Select
                      style={{ height: TOUCH }}
                      placeholder="選機台"
                      options={MACHINES.map((m) => ({ value: m, label: m }))}
                    />
                  </Form.Item>
                </Col>
                <Col xs={24} md={6}>
                  <Form.Item
                    name="包裝產品"
                    label="包裝產品"
                    extra="現在不知道可以先跳過，事後補"
                  >
                    <Select
                      style={{ height: TOUCH }}
                      allowClear
                      placeholder="事後補"
                      options={PRODUCTS.map((p) => ({ value: p, label: p }))}
                    />
                  </Form.Item>
                </Col>
                <Col xs={24} md={6}>
                  <Form.Item name="使用步徑" label="使用步徑（碼表）">
                    <Input style={{ height: TOUCH, fontSize: 18 }} placeholder="例 557" />
                  </Form.Item>
                </Col>
              </Row>
              <Button
                type="primary"
                size="large"
                loading={busy}
                disabled={!chosenLot}
                onClick={submit}
                style={{ height: TOUCH, fontSize: 20, width: "100%" }}
              >
                送出領用
              </Button>
            </Form>
          </Card>
        </>
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
          ? `這一筆已存成紀錄 #${verdict.id}，包含影像與時間。重拍不會讓它消失，請去拿較早進貨的那箱。`
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
