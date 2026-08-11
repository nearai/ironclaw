import type { ComponentProps } from "react";
import { defaultRemarkPlugins, Streamdown } from "streamdown";
import remarkGemoji from "remark-gemoji";

const remarkPlugins = [...Object.values(defaultRemarkPlugins), remarkGemoji];

export function EmojiStreamingMarkdown({
  children,
  ...props
}: ComponentProps<typeof Streamdown>) {
  return (
    <Streamdown {...props} remarkPlugins={remarkPlugins}>
      {children}
    </Streamdown>
  );
}
