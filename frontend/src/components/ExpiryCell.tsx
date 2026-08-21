"use client";

/**
 * A lot's 有效期限, as recorded at receiving.
 *
 * Remaining days are colour-coded because "2027-03-19" does not tell anyone
 * standing at a shelf whether that is a problem.
 */

import { Space, Tag, Tooltip, Typography } from "antd";

const { Text } = Typography;

export default function ExpiryCell({
  date, daysLeft, required,
}: {
  date: string | null;
  daysLeft: number | null;
  required?: boolean;
}) {
  if (!date) {
    return required ? (
      <Tooltip title="這個品項有效期，但這批沒填有效期限">
        <Tag color="orange">未填</Tag>
      </Tooltip>
    ) : (
      <Tooltip title="此品項未標記有效期">
        <Text type="secondary">—</Text>
      </Tooltip>
    );
  }

  const colour = daysLeft == null ? undefined
    : daysLeft < 0 ? "red"
    : daysLeft <= 30 ? "red"
    : daysLeft <= 90 ? "orange"
    : undefined;

  return (
    <Space size={4} wrap>
      <span>{date}</span>
      {daysLeft != null && (
        <Tag color={colour} style={{ marginInlineEnd: 0 }}>
          {daysLeft < 0 ? `已過期 ${-daysLeft} 天` : `剩 ${daysLeft} 天`}
        </Tag>
      )}
    </Space>
  );
}
