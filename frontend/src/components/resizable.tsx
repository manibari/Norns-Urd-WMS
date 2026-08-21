"use client";

/**
 * Drag-to-resize and drag-to-reorder table columns, remembered per table.
 *
 * Remembering is the point. These tables carry a dozen columns behind a
 * horizontal scrollbar, and whoever maintains the master data cares about
 * different columns than whoever is checking stock — so they widen what they
 * need, and having to redo it on every visit would make the feature pointless.
 * Widths live in localStorage: per-person, per-browser, and no server round
 * trip for something this cosmetic.
 *
 * Hand-rolled rather than pulling in react-resizable: it is one pointer-move
 * listener, and a dependency would have to be carried for the life of the app.
 */

import type { TableColumnType } from "antd";
import { useCallback, useEffect, useState } from "react";

const MIN_WIDTH = 60;

type Widths = Record<string, number>;

function widthKey(tableId: string) {
  return `urdwms.colwidths.${tableId}`;
}

function orderKey(tableId: string) {
  return `urdwms.colorder.${tableId}`;
}

export function useColumnWidths(tableId: string) {
  const [widths, setWidths] = useState<Widths>({});
  const [order, setOrder] = useState<string[]>([]);

  useEffect(() => {
    try {
      const raw = localStorage.getItem(widthKey(tableId));
      if (raw) setWidths(JSON.parse(raw));
      const rawOrder = localStorage.getItem(orderKey(tableId));
      if (rawOrder) setOrder(JSON.parse(rawOrder));
    } catch {
      // A corrupt or unavailable store just means defaults.
    }
  }, [tableId]);

  const moveColumn = useCallback((from: string, to: string, keys: string[]) => {
    setOrder(() => {
      const base = keys.slice();
      const fromIndex = base.indexOf(from);
      const toIndex = base.indexOf(to);
      if (fromIndex < 0 || toIndex < 0 || fromIndex === toIndex) return base;
      base.splice(toIndex, 0, ...base.splice(fromIndex, 1));
      try {
        localStorage.setItem(orderKey(tableId), JSON.stringify(base));
      } catch {
        // Not persisting is survivable; the move still applies this session.
      }
      return base;
    });
  }, [tableId]);

  const setWidth = useCallback((key: string, width: number) => {
    setWidths((prev) => {
      const next = { ...prev, [key]: Math.max(MIN_WIDTH, Math.round(width)) };
      try {
        localStorage.setItem(widthKey(tableId), JSON.stringify(next));
      } catch {
        // Not persisting is survivable; the resize still applies this session.
      }
      return next;
    });
  }, [tableId]);

  const reset = useCallback(() => {
    setWidths({});
    setOrder([]);
    try {
      localStorage.removeItem(widthKey(tableId));
      localStorage.removeItem(orderKey(tableId));
    } catch {
      // ignore
    }
  }, [tableId]);

  /** Apply stored order and widths, centre content, and attach drag handles. */
  function resizable<T>(columns: TableColumnType<T>[]): TableColumnType<T>[] {
    const keyOf = (col: TableColumnType<T>, index: number) =>
      String(col.key ?? col.dataIndex ?? index);
    const allKeys = columns.map(keyOf);

    // Stored order first, then anything it does not mention — a column added in
    // a later release must appear rather than vanish because an old saved order
    // predates it.
    const ordered = order.length
      ? [
          ...order.map((k) => columns[allKeys.indexOf(k)]).filter(Boolean),
          ...columns.filter((c, i) => !order.includes(keyOf(c, i))),
        ]
      : columns;

    return ordered.map((col, index) => {
      const key = keyOf(col, columns.indexOf(col) >= 0 ? columns.indexOf(col) : index);
      const width = widths[key] ?? col.width;
      // Centring here rather than on every column definition keeps it one
      // decision instead of a hundred, and an explicit align still wins.
      const centred = { ...col, align: col.align ?? ("center" as const) };
      const heading = (
        <DraggableTitle
          columnKey={key}
          onMove={(from, to) => moveColumn(from, to, ordered.map((c, i) => keyOf(c, i)))}
        >
          {typeof col.title === "function" ? null : col.title}
        </DraggableTitle>
      );
      if (typeof width !== "number") return { ...centred, title: heading };
      return {
        ...centred,
        width,
        title: (
          <ResizableTitle width={width} onResize={(next) => setWidth(key, next)}>
            {heading}
          </ResizableTitle>
        ),
      };
    });
  }

  return {
    resizable,
    reset,
    hasCustomWidths: Object.keys(widths).length > 0 || order.length > 0,
  };
}

function DraggableTitle({
  children, columnKey, onMove,
}: {
  children: React.ReactNode;
  columnKey: string;
  onMove: (from: string, to: string) => void;
}) {
  const [over, setOver] = useState(false);
  return (
    <span
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/column", columnKey);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => {
        // Only react to a column drag; a file dropped on a header should do
        // nothing rather than reorder something.
        if (!e.dataTransfer.types.includes("text/column")) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        const from = e.dataTransfer.getData("text/column");
        setOver(false);
        if (from && from !== columnKey) onMove(from, columnKey);
      }}
      title="拖曳可調整欄位順序"
      style={{
        display: "block", cursor: "grab",
        boxShadow: over ? "inset 2px 0 0 #1677ff" : undefined,
      }}
    >
      {children}
    </span>
  );
}

function ResizableTitle({
  children, width, onResize,
}: {
  children: React.ReactNode;
  width: number;
  onResize: (width: number) => void;
}) {
  const [dragging, setDragging] = useState(false);

  // Mouse events rather than pointer events: pointer capture behaves
  // inconsistently inside antd's sticky/scrolling table header, and mouse
  // events are what every input path here actually produces. Touch is handled
  // separately below for the shop-floor tablet.
  function begin(startX: number) {
    const startWidth = width;
    setDragging(true);

    function move(clientX: number) {
      onResize(startWidth + (clientX - startX));
    }
    function onMouseMove(e: MouseEvent) {
      e.preventDefault();
      move(e.clientX);
    }
    function onTouchMove(e: TouchEvent) {
      if (e.touches[0]) move(e.touches[0].clientX);
    }
    function end() {
      setDragging(false);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", end);
      window.removeEventListener("touchmove", onTouchMove);
      window.removeEventListener("touchend", end);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    }
    // Without this a drag selects the text it passes over instead of resizing.
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", end);
    window.addEventListener("touchmove", onTouchMove, { passive: true });
    window.addEventListener("touchend", end);
  }

  return (
    <span style={{ position: "relative", display: "block", paddingInlineEnd: 10 }}>
      {children}
      <span
        onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); begin(e.clientX); }}
        onTouchStart={(e) => { e.stopPropagation(); begin(e.touches[0]?.clientX ?? 0); }}
        onClick={(e) => e.stopPropagation()}
        title="拖曳調整欄寬"
        style={{
          position: "absolute", insetBlock: -12, insetInlineEnd: -16, width: 22,
          cursor: "col-resize", touchAction: "none", zIndex: 2,
          display: "flex", alignItems: "center", justifyContent: "center",
          background: dragging ? "rgba(22,119,255,0.18)" : "transparent",
        }}
      >
        {/* A visible grip: a resize affordance nobody can see is one nobody uses. */}
        <span
          style={{
            width: 2, height: "60%", borderRadius: 1,
            background: dragging ? "#1677ff" : "#d9d9d9",
          }}
        />
      </span>
    </span>
  );
}
