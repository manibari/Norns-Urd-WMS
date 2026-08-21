"use client";

/**
 * Drag rows into a chosen order, persisted server-side.
 *
 * A handle column rather than a draggable row: every other cell in this table is
 * an input, and making the whole row draggable means every attempt to click into
 * a field starts a drag instead. The handle is the only place that grabs.
 *
 * Row order is saved to the server, not localStorage — unlike column widths.
 * How the master list is grouped (films together, the daily one on top) is a
 * decision about the data that everyone should see; how wide a column is, is
 * one person's reading preference.
 */

import { HolderOutlined } from "@ant-design/icons";
import type { TableColumnType } from "antd";
import { useCallback, useRef, useState } from "react";

const DRAG_TYPE = "text/row-key";

export function useRowDrag<T extends { id: number }>(
  rows: T[],
  onReorder: (orderedIds: number[]) => Promise<void> | void,
) {
  const [dragging, setDragging] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const saving = useRef(false);

  const move = useCallback(async (fromId: number, toId: number) => {
    if (fromId === toId || saving.current) return;
    const ids = rows.map((r) => r.id);
    const from = ids.indexOf(fromId);
    const to = ids.indexOf(toId);
    if (from < 0 || to < 0) return;
    ids.splice(to, 0, ...ids.splice(from, 1));
    saving.current = true;
    try {
      await onReorder(ids);
    } finally {
      saving.current = false;
    }
  }, [rows, onReorder]);

  /** The grab handle, as a first column. */
  function handleColumn(): TableColumnType<T> {
    return {
      title: "",
      key: "__drag",
      width: 44,
      align: "center" as const,
      render: (_: unknown, row: T) => (
        <span
          draggable
          onDragStart={(e) => {
            e.dataTransfer.setData(DRAG_TYPE, String(row.id));
            e.dataTransfer.effectAllowed = "move";
            setDragging(row.id);
          }}
          onDragEnd={() => { setDragging(null); setOver(null); }}
          title="拖曳調整順序"
          style={{
            cursor: "grab", color: dragging === row.id ? "#1677ff" : "#bfbfbf",
            display: "inline-flex", padding: 4,
          }}
        >
          <HolderOutlined />
        </span>
      ),
    };
  }

  /** Row props that make each row a drop target. */
  function rowProps(row: T) {
    return {
      onDragOver: (e: React.DragEvent) => {
        if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
        e.preventDefault();
        setOver(row.id);
      },
      onDragLeave: () => setOver((id) => (id === row.id ? null : id)),
      onDrop: (e: React.DragEvent) => {
        const from = Number(e.dataTransfer.getData(DRAG_TYPE));
        setOver(null);
        setDragging(null);
        if (from) move(from, row.id);
      },
      style: over === row.id && dragging !== row.id
        ? { boxShadow: "inset 0 2px 0 #1677ff" }
        : undefined,
    };
  }

  return { handleColumn, rowProps };
}
