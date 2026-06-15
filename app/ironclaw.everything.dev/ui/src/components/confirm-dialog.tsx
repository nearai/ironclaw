import { useState } from "react";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";

interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: "default" | "destructive";
  onConfirm: () => void;
  isPending?: boolean;
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = "confirm",
  cancelLabel = "cancel",
  variant = "default",
  onConfirm,
  isPending,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            disabled={isPending}
          >
            {cancelLabel}
          </Button>
          <Button
            variant={variant === "destructive" ? "default" : "outline"}
            size="sm"
            onClick={onConfirm}
            disabled={isPending}
            className={
              variant === "destructive"
                ? "bg-destructive text-destructive-foreground hover:bg-destructive/90"
                : ""
            }
          >
            {isPending ? "..." : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function useConfirmDialog() {
  const [open, setOpen] = useState(false);
  const [config, setConfig] = useState<Omit<ConfirmDialogProps, "open" | "onOpenChange">>({
    title: "",
    description: "",
    onConfirm: () => {},
  });

  const confirm = (
    options: Omit<ConfirmDialogProps, "open" | "onOpenChange" | "onConfirm"> & {
      onConfirm: () => void;
    },
  ) => {
    setConfig(options);
    setOpen(true);
  };

  const dialog = <ConfirmDialog open={open} onOpenChange={setOpen} {...config} />;

  return { confirm, dialog };
}
