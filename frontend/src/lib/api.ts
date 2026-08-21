export type Lot = {
  id: number;
  item_id: number;
  item_name: string;
  item_model: string | null;
  item_spec: string | null;
  /** 型號優先，沒型號就顯示名稱 */
  item_label: string;
  receipt_date: string;
  manufacture_date: string | null;
  supplier_lot_code: string | null;
  supplier: string | null;
  /** 驗收單上填的數量原值，保留供稽核比對 */
  entered_meters: number | null;
  entered_unit: string | null;
  /** 標示（有效日期）—— 包材上印的，多數沒有 */
  expiry_date: string | null;
  /** 有效期限（＝收貨時填的那個日期） */
  effective_expiry: string | null;
  /** 這個品項有沒有效期。有的話這批沒填就是漏填 */
  item_has_expiry: number;
  /** 距離到期還幾天。負數＝已過期 */
  days_left: number | null;
  inspection: Record<string, boolean | null>;
  /** 這批被領用過幾次。>0 就不能刪，刪掉會讓領用紀錄指向不存在的批次 */
  draw_count: number;
  /** 合格 | 不合格。不合格不計入在庫、FIFO 不指、領不出來 */
  verdict: string | null;
  recorded_by: string | null;
  confirmed_by: string | null;
  remark: string | null;
  /** 收進來幾箱（不變） */
  qty_received: number;
  /** 已經領走幾箱 */
  qty_drawn: number;
  /** 未動用 | 領貨中 | 已領完 */
  lot_state: "未動用" | "領貨中" | "已領完";
  qty_on_hand: number;
  /** FIFO 指引：唯一該領的那一批 */
  is_fifo_next: boolean;
  /** 同進貨日的其他批：領了也合法，只是指引沒指它 */
  fifo_also_ok: boolean;
};

export type Item = {
  id: number;
  /** 原物料名稱。必填 */
  name: string;
  /** 型號。選填 —— 有些包材（脫氧劑）沒有型號 */
  model: string | null;
  /** 型號優先，沒型號就顯示名稱 */
  label: string;
  /** 標籤上有東西可對映（有型號或箱上料號）。這是「認得出」的先決條件 */
  matchable: boolean;
  /** 是否啟用影像辨識。可自行開關 */
  use_recognition: number;
  /** 實際會不會走辨識 = 有東西可對映 且 已啟用 */
  recognisable: boolean;
  spec: string | null;
  unit: string;
  /** @deprecated 效期改成收貨時直接填有效期限，不再由天數推算 */
  shelf_life_days: number | null;
  safety_stock: number;
  /** 每箱米數。null = 此品項不用米數換算 */
  /** 每箱數量。單位見 pack_unit */
  meters_per_box: number | null;
  /** 計量單位（米／張／包…）。隨品項而定 */
  pack_unit: string | null;
  /** 這個品項有沒有效期。有的話收貨必須填有效期限 */
  has_expiry: number;
  rejected_qty: number;
  /** 最近一次進貨日／建批時間，庫存總覽用來把最新的排在最上面 */
  last_receipt_date: string | null;
  last_lot_at: string | null;
  /** 箱上標籤的完整料號，影像辨識對映用 */
  supplier_code: string | null;
  supplier: string | null;
  on_hand: number;
  /** 在庫換算成米。meters_per_box 為 null 時也是 null */
  on_hand_m: number | null;
  open_lots: number;
};

export type CatalogueEntry = {
  id: number;
  name: string;
  model: string | null;
  label: string;
  recognisable: boolean;
  spec: string | null;
  meters_per_box: number | null;
  on_hand: number;
};

export type Proposal = {
  image_path: string;
  /** 由影像判定的品項。辨識不出時為 null，此時要人工選 */
  item_id: number | null;
  item_name: string | null;
  item_model: string | null;
  item_label: string | null;
  item_match: {
    decision: "lock" | "defer";
    item_id: number | null;
    matched_on: "supplier_code" | "model_in_label" | "manual" | null;
    reason: "no_code_read" | "no_item_match" | "ambiguous_item" | null;
    contenders: string[];
  };
  catalogue: CatalogueEntry[];
  recognition: {
    receipt_date: string | null;
    manufacture_date: string | null;
    /** 型號（T6284BA 這種短碼）—— 對映的關鍵 */
    model_code: string | null;
    /** 箱上完整料號，若標籤上有 */
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
  item_id: number;
  item_name: string | null;
  item_model: string | null;
  item_label: string | null;
  lot_id: number | null;
  /** 這次領幾箱 */
  qty: number;
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
  pending_detail: { id: number; item_id: number; captured_at: string }[];
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
  /** 職位（倉管／廠長／作業員…）。給人看的，跟權限無關 */
  title: string | null;
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
  signers: () => req<{ name: string; title: string | null; role_label: string }[]>("/api/auth/signers"),
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
  lots: (itemId?: number) => req<Lot[]>(`/api/lots${itemId ? `?item_id=${itemId}` : ""}`),
  createLot: (body: Record<string, unknown>) =>
    req<{
      id: number;
      receipt_date: string;
      item_id: number;
      created_item: boolean;
      duplicate_lot_exists: boolean;
      qty: number;
      conversion_note: string | null;
      verdict: string | null;
      same_signer: boolean;
    }>("/api/lots", json(body)),
  patchItem: (id: number, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/items/${id}`, {
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
  /** 辨識不出品項時，人工指定後重新比對批次（不重跑辨識、不重複計費） */
  resolveItem: (itemId: number, ocrReceiptDate: string | null) =>
    req<Partial<Proposal>>("/api/resolve-item", json({ item_id: itemId, ocr_receipt_date: ocrReceiptDate })),
  createScan: (body: Record<string, unknown>) =>
    req<{ id: number; status: Scan["status"]; fifo_expected_date?: string }>("/api/scans", json(body)),
  scans: (status?: string) => req<Scan[]>(`/api/scans${status ? `?status=${status}` : ""}`),
  override: (id: number, reason: string) =>
    req<{ id: number; status: string }>(`/api/scans/${id}/override`, json({ reason })),
  alerts: () => req<Alerts>("/api/alerts"),
  itemOptions: () => req<{ supplier: string[]; material_name: string[]; spec: string[] }>("/api/item-options"),
  roles: () => req<{ code: string; label: string; default_label: string; permissions: string[] }[]>("/api/roles"),
  patchRole: (code: string, label: string) =>
    req<{ code: string; label: string }>(`/api/roles/${code}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ label }),
    }),
  createItem: (body: Record<string, unknown>) => req<Record<string, unknown>>("/api/items", json(body)),
  deleteItem: (id: number) =>
    req<{ id: number; deleted: boolean }>(`/api/items/${id}`, { method: "DELETE" }),
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
  deleteDictEntry: (id: number) =>
    req<{ id: number; deleted: boolean; value: string }>(`/api/dictionary/${id}`, { method: "DELETE" }),
  patchDictEntry: (id: number, body: Record<string, unknown>) =>
    req<Record<string, unknown>>(`/api/dictionary/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
};
