"use client";

/**
 * 庫存總覽的三張圖。手畫的 —— 品項只有個位數，為了幾根長條裝一套圖表庫不划算，
 * 而且自己畫才控得住字級（現場是平板，遠遠看的，預設的圖表字級一律太小）。
 *
 * 三張圖各回答一個問題，不重複：
 *   1. 夠不夠用 —— 在庫 vs 安全水位
 *   2. 老不老   —— 庫齡分佈（依製造日，跟 FIFO 同一把尺）
 *   3. 來不來得及 —— 效期倒數
 */

import { Card, Empty, Space, Tooltip, Typography } from "antd";
import type { Item, Lot } from "@/lib/api";

const { Text } = Typography;

const OK = "#52c41a";
const WARN = "#faad14";
const BAD = "#ff4d4f";
const GREY = "#bfbfbf";

/** 標籤欄寬。夠長的品項名才不會被截掉，又不至於把長條擠到沒地方畫 */
const LABEL = 190;

function Bar({ pct, color }: { pct: number; color: string }) {
  return (
    <div style={{ flex: 1, height: 26, background: "#f0f0f0", borderRadius: 4, position: "relative" }}>
      <div
        style={{
          width: `${Math.max(pct, pct > 0 ? 2 : 0)}%`,
          height: "100%",
          background: color,
          borderRadius: 4,
          transition: "width .3s",
        }}
      />
    </div>
  );
}

/** 在庫 vs 安全水位。長條是在庫量，那根黑線是水位 —— 線在長條右邊就是缺料 */
function StockVsSafety({ items }: { items: Item[] }) {
  const shown = items.filter((i) => i.on_hand > 0 || i.safety_stock > 0);
  if (!shown.length) return <Empty description="尚無庫存" image={Empty.PRESENTED_IMAGE_SIMPLE} />;
  const max = Math.max(...shown.map((i) => Math.max(i.on_hand, i.safety_stock)), 1);

  return (
    <Space orientation="vertical" size={14} style={{ width: "100%" }}>
      {shown.map((item) => {
        const low = item.safety_stock > 0 && item.on_hand < item.safety_stock;
        const tight = !low && item.safety_stock > 0 && item.on_hand < item.safety_stock * 1.5;
        return (
          <div key={item.id} style={{ display: "flex", alignItems: "center", gap: 14 }}>
            <div style={{ width: LABEL, flexShrink: 0, lineHeight: 1.25 }}>
              <div style={{ fontSize: 17, fontWeight: 600 }}>{item.name}</div>
              {item.model && <Text type="secondary" style={{ fontSize: 13 }}>{item.model}</Text>}
            </div>
            <div style={{ flex: 1, position: "relative", display: "flex" }}>
              <Bar pct={(item.on_hand / max) * 100} color={low ? BAD : tight ? WARN : OK} />
              {item.safety_stock > 0 && (
                <Tooltip title={`安全水位 ${item.safety_stock} 箱`}>
                  <div
                    style={{
                      position: "absolute",
                      left: `${(item.safety_stock / max) * 100}%`,
                      top: -4, bottom: -4, width: 3,
                      background: "#262626", borderRadius: 2, cursor: "help",
                    }}
                  />
                </Tooltip>
              )}
            </div>
            <div style={{ width: 108, flexShrink: 0, textAlign: "right" }}>
              <span style={{ fontSize: 24, fontWeight: 700, color: low ? BAD : "#262626" }}>
                {item.on_hand}
              </span>
              <Text type="secondary" style={{ fontSize: 15 }}> 箱</Text>
              {low && <div style={{ fontSize: 13, color: BAD }}>缺 {item.safety_stock - item.on_hand} 箱</div>}
            </div>
          </div>
        );
      })}
      <Text type="secondary" style={{ fontSize: 13 }}>
        黑線＝安全水位。線落在長條右邊就是低於水位。不合格的庫存不計入。
        {items.length > shown.length
          && `另有 ${items.length - shown.length} 個品項無庫存也未設安全水位，沒有東西可畫，未列入。`}
      </Text>
    </Space>
  );
}

/** 庫齡分佈。用製造日算，跟 FIFO 同一把尺 —— 圖上偏紅的那幾箱就是該先領掉的 */
function StockAge({ lots }: { lots: Lot[] }) {
  const live = lots.filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格");
  if (!live.length) return <Empty description="尚無在庫批次" image={Empty.PRESENTED_IMAGE_SIMPLE} />;

  const today = new Date();
  const months = (iso: string) =>
    (today.getTime() - new Date(iso).getTime()) / (1000 * 60 * 60 * 24 * 30.4);

  const BUCKETS = [
    { label: "3 個月內", color: OK, hit: (m: number) => m < 3 },
    { label: "3–6 個月", color: "#95de64", hit: (m: number) => m >= 3 && m < 6 },
    { label: "6–12 個月", color: WARN, hit: (m: number) => m >= 6 && m < 12 },
    { label: "超過一年", color: BAD, hit: (m: number) => m >= 12 },
  ];

  const bars = BUCKETS.map((b) => ({
    label: b.label,
    color: b.color,
    boxes: live
      .filter((l) => l.manufacture_date && b.hit(months(l.manufacture_date)))
      .reduce((s, l) => s + l.qty_on_hand, 0),
  }));
  // 未填製造日單獨一根：它排在 FIFO 最後，跟「很新」不是同一回事，不能混進 3 個月內。
  const undated = live.filter((l) => !l.manufacture_date).reduce((s, l) => s + l.qty_on_hand, 0);
  if (undated > 0) bars.push({ label: "未填製造日", color: GREY, boxes: undated });

  const max = Math.max(...bars.map((b) => b.boxes), 1);

  return (
    <>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 20, height: 210, paddingTop: 8 }}>
        {bars.map((b) => (
          <div key={b.label} style={{ flex: 1, textAlign: "center" }}>
            <div style={{ fontSize: 22, fontWeight: 700, color: b.boxes ? b.color : GREY }}>
              {b.boxes}
            </div>
            <div
              style={{
                height: Math.max((b.boxes / max) * 130, b.boxes ? 6 : 2),
                background: b.boxes ? b.color : "#f0f0f0",
                borderRadius: "4px 4px 0 0",
                margin: "6px auto 0",
                transition: "height .3s",
              }}
            />
            <div style={{ fontSize: 15, marginTop: 8, lineHeight: 1.3 }}>{b.label}</div>
          </div>
        ))}
      </div>
      <Text type="secondary" style={{ fontSize: 13 }}>
        依製造日算庫齡，跟 FIFO 同一把尺。未填製造日的排在 FIFO 最後，所以單獨一根，不併入「3 個月內」。
      </Text>
    </>
  );
}

/** 效期倒數。只有有填有效期限的批次會出現 —— 多數包材沒有，那不是漏填 */
function ExpiryCountdown({ lots }: { lots: Lot[] }) {
  const dated = lots
    .filter((l) => l.qty_on_hand > 0 && l.verdict !== "不合格" && l.effective_expiry && l.days_left != null)
    .sort((a, b) => (a.days_left ?? 0) - (b.days_left ?? 0))
    .slice(0, 8);
  if (!dated.length) return null;

  const max = Math.max(...dated.map((l) => Math.max(l.days_left ?? 0, 0)), 1);

  return (
    <Card title={<span style={{ fontSize: 18 }}>效期倒數（剩最少的在最上面）</span>} style={{ marginTop: 24 }}>
      <Space orientation="vertical" size={14} style={{ width: "100%" }}>
        {dated.map((lot) => {
          const days = lot.days_left ?? 0;
          const color = days < 0 ? BAD : days <= 90 ? WARN : OK;
          return (
            <div key={lot.id} style={{ display: "flex", alignItems: "center", gap: 14 }}>
              <div style={{ width: LABEL, flexShrink: 0, lineHeight: 1.25 }}>
                <div style={{ fontSize: 17, fontWeight: 600 }}>{lot.item_name}</div>
                <Text type="secondary" style={{ fontSize: 13 }}>製造 {lot.manufacture_date ?? "未填"}</Text>
              </div>
              <Bar pct={(Math.max(days, 0) / max) * 100} color={color} />
              <div style={{ width: 170, flexShrink: 0, textAlign: "right" }}>
                <span style={{ fontSize: 24, fontWeight: 700, color }}>
                  {days < 0 ? `已過期 ${-days}` : days}
                </span>
                <Text type="secondary" style={{ fontSize: 15 }}> 天</Text>
                <div style={{ fontSize: 13, color: "#8c8c8c" }}>
                  {lot.effective_expiry}．{lot.qty_on_hand} 箱
                </div>
              </div>
            </div>
          );
        })}
      </Space>
    </Card>
  );
}

export default function StockCharts({ items, lots }: { items: Item[]; lots: Lot[] }) {
  return (
    <>
      <div style={{ display: "flex", gap: 24, flexWrap: "wrap", marginTop: 24 }}>
        <Card
          title={<span style={{ fontSize: 18 }}>在庫 vs 安全水位</span>}
          style={{ flex: "1 1 560px", minWidth: 0 }}
        >
          <StockVsSafety items={items} />
        </Card>
        <Card
          title={<span style={{ fontSize: 18 }}>庫齡分佈（依製造日）</span>}
          style={{ flex: "1 1 420px", minWidth: 0 }}
        >
          <StockAge lots={lots} />
        </Card>
      </div>
      <ExpiryCountdown lots={lots} />
    </>
  );
}
