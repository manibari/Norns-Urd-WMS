export type Lot = {
  id: number;
  item_code: string;
  item_name: string;
  receipt_date: string;
  manufacture_date: string | null;
  supplier_lot_code: string | null;
  supplier: string | null;
  /** 驗收單上填的數量原值，保留供稽核比對 */
  entered_meters: number | null;
  entered_unit: string | null;
  expiry_date: string | null;
  inspection: Record<string, boolean | null>;
  /** 這批被領用過幾次。>0 就不能刪，刪掉會讓領用紀錄指向不存在的批次 */
  draw_count: number;
  /** 合格 | 不合格。不合格不計入在庫、FIFO 不指、領不出來 */
  verdict: string | null;
  recorded_by: string | null;
  confirmed_by: string | null;
  remark: string | null;
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
  rejected_qty: number;
  /** 箱上標籤的完整料號，影像辨識對映用 */
  supplier_code: string | null;
  supplier: string | null;
  on_hand: number;
  /** 在庫換算成米。meters_per_box 為 null 時也是 null */
  on_hand_m: number | null;
  open_lots: number;
};

export type CatalogueEntry = {
  item_code: string;
  name: string;
  spec: string | null;
  meters_per_box: number | null;
  on_hand: number;
};

export type Proposal = {
  image_path: string;
  /** 由影像判定的型號。辨識不出時為 null，此時要人工選 */
  item_code: string | null;
  item_name: string | null;
  item_match: {
    decision: "lock" | "defer";
    item_code: string | null;
    matched_on: "supplier_code" | "model_in_label" | "manual" | null;
    reason: "no_code_read" | "no_item_match" | "ambiguous_item" | null;
    contenders: string[];
  };
  catalogue: CatalogueEntry[];
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
  /** 標籤讀到的料號跟所選型號對不對得上。null = 讀不到料號，無從判斷 */
  expected_supplier_code: string | null;
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
  rejected: (Lot & { name: string })[];
};

export type AuditEntry = {
  id: number;
  at: string;
  actor: string;
  action: string;
  detail: Record<string, unknown>;
};

export type DictEntry = {
  id: number;
  category: string;
  value: string;
  sort_order: number;
  active: number;
};

export type Dictionary = {
  categories: Record<string, string>;
  entries: Record<string, DictEntry[]>;
};

export type SessionUser = {
  id: number;
  username: string;
  /** 顯示名／簽核名 —— 會寫進紀錄人與稽核軌跡 */
  name: string;
  role: string;
  role_label: string;
  must_change: boolean;
  permissions: string[];
};

const TOKEN_KEY = "urdwms.token";

export const token = {
  get: () => (typeof window === "undefined" ? null : localStorage.getItem(TOKEN_KEY)),
  set: (value: string) => localStorage.setItem(TOKEN_KEY, value),
  clear: () => localStorage.removeItem(TOKEN_KEY),
};

/** Thrown on 401 so the shell can send the user back to the PIN pad. */
export class Unauthenticated extends Error {}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const auth = token.get();
  const res = await fetch(path, {
    cache: "no-store",
    ...init,
    headers: {
      ...(init?.headers ?? {}),
      ...(auth ? { Authorization: `Bearer ${auth}` } : {}),
    },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ detail: res.statusText }));
    if (res.status === 401) {
      token.clear();
      throw new Unauthenticated(body.detail ?? "請先登入");
    }
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
  signers: () => req<{ name: string; role_label: string }[]>("/api/auth/signers"),
  login: (username: string, password: string) =>
    req<{ token: string; expires_at: string; user: SessionUser }>(
      "/api/auth/login", json({ username, password })),
  changePassword: (current_password: string, new_password: string) =>
    req<{ ok: boolean }>("/api/auth/password", json({ current_password, new_password })),
  logout: () => req<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),
  me: () => req<SessionUser>("/api/auth/me"),
  users: () => req<(SessionUser & { active: number; created_at: string })[]>("/api/users"),
  createUser: (body: Record<string, unknown>) => req<Record<string, unknown>>("/api/users", json(body)),
  patchUser: (id: number, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/users/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  items: () => req<Item[]>("/api/items"),
  lots: (itemCode?: string) =>
    req<Lot[]>(`/api/lots${itemCode ? `?item_code=${encodeURIComponent(itemCode)}` : ""}`),
  createLot: (body: Record<string, unknown>) =>
    req<{
      id: number;
      receipt_date: string;
      created_item: boolean;
      duplicate_lot_exists: boolean;
      qty: number;
      conversion_note: string | null;
      verdict: string | null;
      same_signer: boolean;
    }>("/api/lots", json(body)),
  patchItem: (code: string, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/items/${encodeURIComponent(code)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  /** 只傳影像：型號與批次都由辨識判定 */
  recognize: (file: File) => {
    const form = new FormData();
    form.append("image", file);
    return req<Proposal>("/api/recognize", { method: "POST", body: form });
  },
  /** 辨識不出型號時，人工指定後重新比對批次（不重跑辨識、不重複計費） */
  resolveItem: (itemCode: string, ocrReceiptDate: string | null) =>
    req<Partial<Proposal>>("/api/resolve-item", json({ item_code: itemCode, ocr_receipt_date: ocrReceiptDate })),
  createScan: (body: Record<string, unknown>) =>
    req<{ id: number; status: Scan["status"]; fifo_expected_date?: string }>("/api/scans", json(body)),
  scans: (status?: string) => req<Scan[]>(`/api/scans${status ? `?status=${status}` : ""}`),
  override: (id: number, reason: string) =>
    req<{ id: number; status: string }>(`/api/scans/${id}/override`, json({ reason })),
  alerts: () => req<Alerts>("/api/alerts"),
  createItem: (body: Record<string, unknown>) => req<Record<string, unknown>>("/api/items", json(body)),
  deleteItem: (code: string) =>
    req<{ item_code: string; deleted: boolean }>(`/api/items/${encodeURIComponent(code)}`, { method: "DELETE" }),
  patchLot: (id: number, body: Record<string, unknown>) =>
    req<{ id: number; changed: Record<string, unknown>; posted_draws: number }>(`/api/lots/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  deleteLot: (id: number) =>
    req<{ id: number; deleted: boolean }>(`/api/lots/${id}`, { method: "DELETE" }),
  audit: (action?: string) =>
    req<AuditEntry[]>(`/api/audit${action ? `?action=${action}` : ""}`),
  dictionary: (includeInactive = false) =>
    req<Dictionary>(`/api/dictionary${includeInactive ? "?include_inactive=true" : ""}`),
  addDictEntry: (category: string, value: string) =>
    req<{ id: number; value: string; revived: boolean }>("/api/dictionary", json({ category, value })),
  patchDictEntry: (id: number, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/dictionary/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
};
