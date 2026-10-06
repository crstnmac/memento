import { useEffect, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Textarea } from "@/components/ui/textarea";

interface NewMemoryDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (text: string) => Promise<void>;
}

export function NewMemoryDialog({
  open,
  onOpenChange,
  onSave,
}: NewMemoryDialogProps) {
  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      onOpenChange(false);
    };
    document.addEventListener("keydown", closeOnEscape, true);
    return () => document.removeEventListener("keydown", closeOnEscape, true);
  }, [open, onOpenChange]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const text = String(data.get("note") ?? "").trim();
    if (text) {
      await onSave(text);
      onOpenChange(false);
      event.currentTarget.reset();
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>New memory</DialogTitle>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="note">What are you working on?</FieldLabel>
            <Textarea
              id="note"
              name="note"
              placeholder="A note, thread, decision, or open loop…"
              className="min-h-28"
              autoFocus
            />
          </Field>
          <DialogFooter className="mt-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit">Save</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
