import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../utils/cn";

type PageScrollProps = HTMLAttributes<HTMLDivElement> & {
  contained?: boolean;
  contentClassName?: string;
  overlay?: ReactNode;
  scrollClassName?: string;
};

export function PageScroll({
  children,
  className = "",
  contentClassName = "",
  contained = false,
  overlay = null,
  scrollClassName = "",
  ...rest
}: PageScrollProps) {
  const content = (
    <div className={cn("v2-page-entrance flex-1 p-4 sm:p-6", contentClassName)}>
      {children}
    </div>
  );

  if (contained) {
    const scroller = (
      <div className={cn("min-h-0 flex-1 overflow-y-auto", scrollClassName)}>
        {content}
      </div>
    );
    return (
      <div
        className={cn("flex h-full min-h-0 flex-col overflow-hidden", className)}
        {...rest}
      >
        {overlay ? (<>{scroller}{overlay}</>) : scroller}
      </div>
    );
  }

  return (
    <div
      className={cn("flex h-full flex-col overflow-y-auto", scrollClassName, className)}
      {...rest}
    >
      {overlay ? (<>{content}{overlay}</>) : content}
    </div>
  );
}

export function PageStack({
  children,
  className = "",
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("space-y-5", className)} {...rest}>
      {children}
    </div>
  );
}
