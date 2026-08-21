"use client";

/**
 * Issuing screen — the one operators use on the packing line.
 *
 * The photo decides everything it can: which 型號 the box is, and which lot.
 * The operator is asked only for what a camera cannot know (上/下膜, machine,
 * product, meter reading) — and for the 型號 only when recognition could not
 * settle it. Making someone pick a 型號 they are holding in their hands is the
 * kind of step that gets skipped, and a skipped step is how the paper form
 * comes back.
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

import { CameraOutlined, InboxOutlined, ReloadOutlined } from "@ant-design/icons";
import {
  Alert, Button, Card, Col, Empty, Form, Input, Radio,
  Row, Select, Space, Spin, Tag, Typography, message,
} from "antd";
import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, type Dictionary, type Proposal } from "@/lib/api";

const { Title, Text } = Typography;

const TOUCH = 64;

const ITEM_DEFER_COPY: Record<string, string> = {
  no_code_read: "標籤上的料號讀不出來",
  no_item_match: "標籤上的料號不在品項主檔裡",
  ambiguous_item: "標籤同時對到多個型號，無法確定是哪一個",
};

type Verdict = { status: string; id: number; expected?: string } | null;

export default function IssuePage() {
  const [busy, setBusy] = useState(false);
  // Machines and products are this factory's configuration (基本資料), not the
  // product's — the paper form lists them in its own notes.
  const [dict, setDict] = useState<Dictionary | null>(null);
  useEffect(() => { api.dictionary().then(setDict).catch(() => undefined); }, []);
  const machines = (dict?.entries.machine ?? []).map((e) => e.value);
  const products = (dict?.entries.packed_product ?? []).map((e) => e.value);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [chosenLot, setChosenLot] = useState<number>();
  const [verdict, setVerdict] = useState<Verdict>(null);
  const [form] = Form.useForm();
  const fileRef = useRef<HTMLInputElement>(null);

  const fifoTarget = useMemo(() => {
    if (!proposal?.candidates.length) return null;
    return proposal.candidates.reduce((a, b) => (a.receipt_date <= b.receipt_date ? a : b));
  }, [proposal]);

  async function onCapture(file: File) {
    setBusy(true);
    setVerdict(null);
    try {
      const result = await api.recognize(file);
      setProposal(result);
      setChosenLot(result.locked_lot?.lot_id);
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function pickItem(itemCode: string) {
    if (!proposal) return;
    setBusy(true);
    try {
      const patch = await api.resolveItem(itemCode, proposal.recognition.receipt_date);
      const next = { ...proposal, ...patch } as Proposal;
      setProposal(next);
      setChosenLot(next.locked_lot?.lot_id);
    } catch (e) {
      message.error((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function submit() {
    if (!proposal?.item_code) return;
    const fields = await form.validateFields().catch(() => null);
    if (!fields) return;
    setBusy(true);
    try {
      const res = await api.createScan({
        item_code: proposal.item_code,
        lot_id: chosenLot ?? null,
        image_path: proposal.image_path,
        ocr_receipt_date: proposal.recognition.receipt_date,
        ocr_confidence: proposal.recognition.confidence,
        ocr_notes: proposal.recognition.notes,
        match_distance: proposal.match_distance,
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
    setProposal(null);
    setChosenLot(undefined);
    setVerdict(null);
    form.resetFields();
  }

  if (verdict) return <VerdictBlock verdict={verdict} onNext={reset} />;

  const itemDeferred = proposal && !proposal.item_code;

  return (
    <>
      <Title level={3} style={{ marginTop: 0 }}>領用登錄</Title>

      <Card title="1. 拍箱子">
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
          loading={busy && !proposal}
          onClick={() => fileRef.current?.click()}
          style={{ height: TOUCH, fontSize: 20, width: "100%" }}
        >
          {proposal ? "重拍" : "拍照或選擇照片"}
        </Button>
        <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
          對準側面的標籤和紅色驗收章。型號和批次都由照片判斷，不用先選。
        </Text>
      </Card>

      {busy && !proposal && (
        <Card style={{ marginTop: 24, textAlign: "center" }} title="辨識中">
          <Spin size="large" />
          <Text type="secondary" style={{ display: "block", marginTop: 12 }}>
            正在讀取標籤料號與驗收章日期
          </Text>
        </Card>
      )}

      {proposal && (
        <>
          <Card
            title="2. 確認"
            style={{ marginTop: 24 }}
            extra={<Button icon={<ReloadOutlined />} onClick={reset}>重來</Button>}
          >
            <Row gutter={24}>
              <Col xs={24} md={10}>
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img
                  src={proposal.image_path}
                  alt="領用箱影像"
                  style={{ width: "100%", borderRadius: 6, border: "1px solid #f0f0f0" }}
                />
                <Text type="secondary" style={{ display: "block", marginTop: 8, fontSize: 12 }}>
                  標籤讀到：{proposal.recognition.item_code ?? "讀不出料號"}
                  {proposal.recognition.receipt_date && `｜章：${proposal.recognition.receipt_date}`}
                </Text>
              </Col>

              <Col xs={24} md={14}>
                {itemDeferred ? (
                  <>
                    <Alert
                      type="warning"
                      title="辨識不出是哪個型號，請自己選"
                      description={
                        (ITEM_DEFER_COPY[proposal.item_match.reason ?? ""] ?? "無法判定型號") +
                        (proposal.item_match.contenders.length
                          ? `（可能是：${proposal.item_match.contenders.join("、")}）`
                          : "")
                      }
                      style={{ marginBottom: 16 }}
                    />
                    <Select
                      size="large"
                      style={{ width: "100%", height: TOUCH }}
                      placeholder="選型號"
                      loading={busy}
                      onChange={pickItem}
                      options={proposal.catalogue.map((c) => ({
                        value: c.item_code,
                        label: `${c.item_code}｜${c.name}（在庫 ${c.on_hand}）`,
                        disabled: c.on_hand === 0,
                      }))}
                    />
                    <Text type="secondary" style={{ display: "block", marginTop: 8 }}>
                      在庫 0 的型號不能選 —— 沒有批次可以領。
                    </Text>
                  </>
                ) : (
                  <>
                    <div style={{ marginBottom: 16 }}>
                      <Text type="secondary">型號</Text>
                      <div>
                        <Text strong style={{ fontSize: 28 }}>{proposal.item_code}</Text>{" "}
                        <Text style={{ fontSize: 18 }}>{proposal.item_name}</Text>{" "}
                        <Tag color={proposal.item_match.matched_on === "manual" ? "orange" : "blue"}>
                          {proposal.item_match.matched_on === "manual" ? "人工指定" : "影像判定"}
                        </Tag>
                      </div>
                    </div>

                    {proposal.decision === "lock" ? (
                      <Alert
                        type="info"
                        title={`辨識到進貨日 ${proposal.recognition.receipt_date}`}
                        description={`信心 ${(proposal.recognition.confidence * 100).toFixed(0)}%，已比對到在庫批次。請確認這是你手上那一箱。`}
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

                    <Text strong style={{ fontSize: 16 }}>在庫批次（請確認你手上這箱）</Text>
                    <Radio.Group
                      value={chosenLot}
                      onChange={(e) => setChosenLot(e.target.value)}
                      style={{ display: "block", marginTop: 12 }}
                    >
                      <Space orientation="vertical" style={{ width: "100%" }} size={12}>
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
                            <Space wrap>
                              <span style={{ fontSize: 18 }}>進貨 {c.receipt_date}</span>
                              <Text type="secondary">製造 {c.manufacture_date ?? "—"}</Text>
                              <Text type="secondary">在庫 {c.qty_on_hand}</Text>
                              {fifoTarget?.lot_id === c.lot_id && <Tag color="green">FIFO 應領</Tag>}
                            </Space>
                          </Radio>
                        ))}
                      </Space>
                    </Radio.Group>
                    {!proposal.candidates.length && (
                      <Empty
                        image={Empty.PRESENTED_IMAGE_SIMPLE}
                        description={`${proposal.item_code} 沒有可領用的在庫批次`}
                      >
                        <Link href="/">
                          <Button type="primary" icon={<InboxOutlined />}>去收貨建批</Button>
                        </Link>
                      </Empty>
                    )}
                  </>
                )}
              </Col>
            </Row>
          </Card>

          {!itemDeferred && proposal.candidates.length > 0 && (
            <Card title="3. 填領用資訊" style={{ marginTop: 24 }}>
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
                        options={machines.map((m) => ({ value: m, label: m }))}
                      />
                    </Form.Item>
                  </Col>
                  <Col xs={24} md={6}>
                    <Form.Item name="產品名稱" label="產品名稱" extra="現在不知道可以先跳過，事後補">
                      <Select
                        style={{ height: TOUCH }}
                        allowClear
                        placeholder="事後補"
                        options={products.map((p) => ({ value: p, label: p }))}
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
          )}
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
