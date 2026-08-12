import React from "react";
import { cn } from "../utils/cn";

const MARQUEE_GAP_PX = 24;
const MARQUEE_PIXELS_PER_SECOND = 36;

type MarqueeStyle = React.CSSProperties & {
  "--marquee-distance"?: string;
  "--marquee-duration"?: string;
};

export function MarqueeText({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const viewportRef = React.useRef<HTMLSpanElement>(null);
  const textRef = React.useRef<HTMLSpanElement>(null);
  const [metrics, setMetrics] = React.useState({ overflow: false, distance: 0 });

  const measure = React.useCallback(() => {
    const viewport = viewportRef.current;
    const text = textRef.current;
    if (!viewport || !text) return;

    const textWidth = text.scrollWidth;
    const overflow = textWidth > viewport.clientWidth + 1;
    const distance = overflow ? textWidth + MARQUEE_GAP_PX : 0;
    setMetrics((current) => (
      current.overflow === overflow && current.distance === distance
        ? current
        : { overflow, distance }
    ));
  }, []);

  React.useLayoutEffect(() => {
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    if (viewportRef.current) observer.observe(viewportRef.current);
    if (textRef.current) observer.observe(textRef.current);
    return () => observer.disconnect();
  }, [children, measure]);

  const style: MarqueeStyle | undefined = metrics.overflow
    ? {
        "--marquee-distance": `${metrics.distance}px`,
        "--marquee-duration": `${Math.max(4, metrics.distance / MARQUEE_PIXELS_PER_SECOND)}s`,
      }
    : undefined;

  return (
    <span
      ref={viewportRef}
      className={cn("v2-marquee min-w-0 overflow-hidden", className)}
      data-marquee-overflow={metrics.overflow ? "true" : "false"}
      style={style}
    >
      <span className="v2-marquee-track">
        <span ref={textRef} className="v2-marquee-text">{children}</span>
        {metrics.overflow && (
          <span aria-hidden="true" className="v2-marquee-copy">{children}</span>
        )}
      </span>
    </span>
  );
}
