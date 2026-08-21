"use client";

/**
 * A dropdown of values already in use, with an escape hatch to add a new one.
 *
 * Free text on these fields is how a master list rots: 希悅爾 / 臺灣希悅爾 /
 * 希悅爾公司 become three suppliers, and every report that groups by supplier is
 * quietly wrong from then on. A plain dropdown would be worse — a new film or a
 * new supplier genuinely does appear, and a form that cannot accept one sends
 * people back to paper.
 *
 * So: pick from what exists (the common case, one tap), or add deliberately.
 */

import { PlusOutlined } from "@ant-design/icons";
import { Button, Divider, Input, Select, Space, message } from "antd";
import { useRef, useState } from "react";
import { api } from "@/lib/api";

type Props = {
  value?: string;
  onChange?: (value: string) => void;
  options: string[];
  placeholder?: string;
  addLabel?: string;
  /** Render richer labels for known values (e.g. 型號 with its 品名) */
  describe?: (value: string) => string;
  /**
   * Dictionary category to persist a newly added value into. Without it, a value
   * typed here exists only until the page reloads — and the next person types a
   * slightly different spelling, which is the exact problem dropdowns solve.
   */
  category?: string;
  onAdded?: () => void;
  disabledWhen?: (value: string) => boolean;
  style?: React.CSSProperties;
  size?: "small" | "middle" | "large";
};

export default function CreatableSelect({
  value, onChange, options, placeholder, addLabel = "新增", describe, disabledWhen,
  style, size, category, onAdded,
}: Props) {
  const [draft, setDraft] = useState("");
  const [extra, setExtra] = useState<string[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);

  const all = Array.from(new Set([...options, ...extra, ...(value ? [value] : [])]));

  async function add() {
    const next = draft.trim();
    if (!next) return;
    if (!all.includes(next)) setExtra((prev) => [...prev, next]);
    setDraft("");
    onChange?.(next);
    if (!category) return;
    try {
      await api.addDictEntry(category, next);
      onAdded?.();
    } catch (e) {
      // Already-exists is fine: the value is selectable either way. Anything
      // else means it will not persist, and the operator should know now rather
      // than find a missing option tomorrow.
      const text = (e as Error).message;
      if (!text.includes("已存在")) message.warning(`「${next}」未存進字典：${text}`);
    }
  }

  return (
    <Select
      value={value}
      onChange={onChange}
      placeholder={placeholder}
      showSearch
      allowClear
      size={size}
      style={style}
      filterOption={(input, option) =>
        String(option?.value ?? "").toLowerCase().includes(input.toLowerCase())
        || String(option?.label ?? "").toLowerCase().includes(input.toLowerCase())
      }
      options={all.map((v) => ({
        value: v,
        label: describe ? describe(v) : v,
        disabled: disabledWhen?.(v) ?? false,
      }))}
      popupRender={(menu) => (
        <>
          {menu}
          <Divider style={{ margin: "8px 0" }} />
          <Space style={{ padding: "0 8px 4px" }}>
            <Input
              ref={inputRef as never}
              placeholder={addLabel}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.stopPropagation()}
              onPressEnter={add}
            />
            <Button type="text" icon={<PlusOutlined />} onClick={add}>
              {addLabel}
            </Button>
          </Space>
        </>
      )}
    />
  );
}
