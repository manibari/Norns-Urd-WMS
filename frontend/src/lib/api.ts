export type Lot = {
  id: number;
  item_code: string;
  item_name: string;
  receipt_date: string;
  manufacture_date: string | null;
  supplier_lot_code: string | null;
  qty_on_hand: number;
  is_fifo_next: boolean;
};

export type Item = {
  item_code: string;
  name: string;
  spec: string | null;
  unit: string;
  shelf_life_days: number | null;
  safety_stock: number;
  /** 每箱米數。null = 此品項不用米數換算 */
  meters_per_box: number | null;
  on_hand: number;
  /** 在庫換算成米。meters_per_box 為 null 時也是 null */
  on_hand_m: number | null;
  open_lots: number;
};

export type Proposal = {
  image_path: string;
  item_code: string;
  item_name: string | null;
  recognition: {
    receipt_date: string | null;
    manufacture_date: string | null;
    item_code: string | null;
    confidence: number;
    stamp_visible: boolean;
    notes: string;
    error: string | null;
  };
  candidates: { lot_id: number; receipt_date: string; manufacture_date: string | null; qty_on_hand: number }[];
  decision: "lock" | "defer";
  defer_reason: string | null;
  match_distance: number | null;
  locked_lot: { lot_id: number; receipt_date: string; manufacture_date: string | null; qty_on_hand: number } | null;
  fifo_ok: boolean | null;
  fifo_expected_date: string | null;
};

export type Scan = {
  id: number;
  item_code: string;
  item_name: string | null;
  lot_id: number | null;
  status: "posted" | "blocked_fifo" | "blocked_unreadable" | "overridden" | "voided";
  captured_at: string;
  captured_by: string;
  image_path: string | null;
  ocr_receipt_date: string | null;
  ocr_confidence: number | null;
  receipt_date: string | null;
  manufacture_date: string | null;
  fifo_expected_date: string | null;
  field_values: Record<string, string>;
  detail_pending: number;
  override_by: string | null;
  override_reason: string | null;
};

export type Alerts = {
  checked_at: string;
  total: number;
  thresholds: { expiry_days: number; stale_days: number; pending_hours: number };
  expiring: (Lot & { expires_on: string; days_left: number; name: string })[];
  stale: (Lot & { age_days: number; name: string })[];
  low_stock: { item_code: string; name: string; safety_stock: number; on_hand: number }[];
  pending_detail: { id: number; item_code: string; captured_at: string }[];
};

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, { cache: "no-store", ...init });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ detail: res.statusText }));
    throw new Error(body.detail ?? "請求失敗");
  }
  return res.json();
}

const json = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
});

export const api = {
  items: () => req<Item[]>("/api/items"),
  lots: (itemCode?: string) =>
    req<Lot[]>(`/api/lots${itemCode ? `?item_code=${encodeURIComponent(itemCode)}` : ""}`),
  createLot: (body: Record<string, unknown>) =>
    req<{
      id: number;
      receipt_date: string;
      created_item: boolean;
      same_day_lot_exists: boolean;
      qty: number;
      conversion_note: string | null;
    }>("/api/lots", json(body)),
  patchItem: (code: string, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/items/${encodeURIComponent(code)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  recognize: (itemCode: string, file: File) => {
    const form = new FormData();
    form.append("item_code", itemCode);
    form.append("image", file);
    return req<Proposal>("/api/recognize", { method: "POST", body: form });
  },
  createScan: (body: Record<string, unknown>) =>
    req<{ id: number; status: Scan["status"]; fifo_expected_date?: string }>("/api/scans", json(body)),
  scans: (status?: string) => req<Scan[]>(`/api/scans${status ? `?status=${status}` : ""}`),
  override: (id: number, reason: string) =>
    req<{ id: number; status: string }>(`/api/scans/${id}/override`, json({ reason })),
  alerts: () => req<Alerts>("/api/alerts"),
};
