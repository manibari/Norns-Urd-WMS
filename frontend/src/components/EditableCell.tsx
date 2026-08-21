"use client";

/**
 * One directly-editable cell.
 *
 * Uncontrolled on purpose: a controlled input would re-render the whole table on
 * every keystroke, which on a 40-row master list is felt. The value is committed
 * on blur, and `version` in the key remounts the cell when the row is refreshed
 * from the server so an outside change is not silently kept out by stale DOM.
 *
 * Saving on blur rather than behind an edit/save button is the point: this is
 * master data someone maintains the way they would in a spreadsheet, and a
 * modal per field turns a five-minute tidy-up into an afternoon.
 */

import { Input, InputNumber } from "antd";
import { useState } from "react";

type Base = {
  onSave: (value: string | number | null) => Promise<void> | void;
  placeholder?: string;
  disabled?: boolean;
};

export function TextCell({
  value, onSave, placeholder, disabled, strong,
}: Base & { value: string | null; strong?: boolean }) {
  const [busy, setBusy] = useState(false);
  return (
    <Input
      key={value ?? ""}
      defaultValue={value ?? ""}
      placeholder={placeholder}
      disabled={disabled || busy}
      variant="borderless"
      style={{ paddingInline: 4, fontWeight: strong ? 600 : undefined }}
      onBlur={async (e) => {
        const next = e.target.value.trim();
        if (next === (value ?? "")) return;
        setBusy(true);
        try {
          await onSave(next || null);
        } finally {
          setBusy(false);
        }
      }}
      onPressEnter={(e) => (e.target as HTMLInputElement).blur()}
    />
  );
}

export function NumberCell({
  value, onSave, placeholder, disabled, min = 0, step = 1, suffix,
}: Base & { value: number | null; min?: number; step?: number; suffix?: string }) {
  const [busy, setBusy] = useState(false);
  return (
    <InputNumber
      key={String(value ?? "")}
      defaultValue={value ?? undefined}
      placeholder={placeholder}
      disabled={disabled || busy}
      min={min}
      step={step}
      variant="borderless"
      style={{ width: "100%" }}
      formatter={suffix ? (v) => (v === undefined || v === "" ? "" : `${v} ${suffix}`) : undefined}
      parser={suffix ? ((v) => (v ?? "").replace(` ${suffix}`, "") as unknown as number) : undefined}
      onBlur={async (e) => {
        const raw = e.target.value.replace(suffix ? ` ${suffix}` : "", "").replace(/,/g, "").trim();
        const next = raw === "" ? null : Number(raw);
        if (next === value || (next !== null && Number.isNaN(next))) return;
        setBusy(true);
        try {
          await onSave(next);
        } finally {
          setBusy(false);
        }
      }}
      onPressEnter={(e) => (e.target as HTMLInputElement).blur()}
    />
  );
}
