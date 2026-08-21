"use client";

/**
 * Drag-to-resize table columns, with the widths remembered per table.
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

function storageKey(tableId: string) {
  return `urdwms.colwidths.${tableId}`;
}

export function useColumnWidths(tableId: string) {
  const [widths, setWidths] = useState<Widths>({});

  useEffect(() => {
    try {
      const raw = localStorage.getItem(storageKey(tableId));
      if (raw) setWidths(JSON.parse(raw));
    } catch {
      // A corrupt or unavailable store just means default widths.
    }
  }, [tableId]);

  const setWidth = useCallback((key: string, width: number) => {
    setWidths((prev) => {
      const next = { ...prev, [key]: Math.max(MIN_WIDTH, Math.round(width)) };
      try {
        localStorage.setItem(storageKey(tableId), JSON.stringify(next));
      } catch {
        // Not persisting is survivable; the resize still applies this session.
      }
      return next;
    });
  }, [tableId]);

  const reset = useCallback(() => {
    setWidths({});
    try {
      localStorage.removeItem(storageKey(tableId));
    } catch {
      // ignore
    }
  }, [tableId]);

  /** Apply stored widths, centre content, and attach a drag handle. */
  function resizable<T>(columns: TableColumnType<T>[]): TableColumnType<T>[] {
    return columns.map((col, index) => {
      const key = String(col.key ?? col.dataIndex ?? index);
      const width = widths[key] ?? col.width;
      // Centring here rather than on every column definition keeps it one
      // decision instead of a hundred, and an explicit align still wins.
      const centred = { ...col, align: col.align ?? ("center" as const) };
      if (typeof width !== "number") return centred;
      return {
        ...centred,
        width,
        title: (
          <ResizableTitle
            width={width}
            onResize={(next) => setWidth(key, next)}
          >
            {typeof col.title === "function" ? null : col.title}
          </ResizableTitle>
        ),
      };
    });
  }

  return { resizable, reset, hasCustomWidths: Object.keys(widths).length > 0 };
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
