import { useCallback, useEffect, useState } from "react";
import { BookOpenIcon, FolderOpenIcon, RefreshCwIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldSet,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import { useMemento } from "@/lib/store";
import type { VaultStatus } from "@/lib/types";

export function VaultSection() {
  const { settings, saveSetting, refreshAll } = useMemento();
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => setStatus(await api.vaultStatus()), []);

  useEffect(() => {
    refresh().catch((error) => setMessage(String(error)));
  }, [refresh]);

  const sync = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const report = await api.vaultSyncNow();
      await Promise.all([refresh(), refreshAll()]);
      const changed = report.exported + report.imported + report.updated;
      setMessage(
        changed === 0
          ? "Vault is already up to date."
          : `Synced ${report.exported} generated files, imported ${report.imported} notes, and updated ${report.updated} notes.`,
      );
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <FieldSet>
      <div className="mb-1 flex items-center gap-2 text-base font-medium">
        <BookOpenIcon className="size-4" />
        Obsidian memory vault
      </div>
      <FieldDescription className="mb-3">
        A readable Markdown mirror of captured memories, meetings, and daily
        indexes. Markdown placed in the notes folder is ingested back into
        Memento automatically.
      </FieldDescription>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="vault-path">Vault location</FieldLabel>
          <FieldDescription>
            Leave empty to use Memento&apos;s private application-data folder.
            Enter an absolute path to use an existing Obsidian vault.
          </FieldDescription>
          <Input
            id="vault-path"
            value={settings.vaultPath}
            placeholder={status?.path ?? "/Users/you/Documents/Memento Memory"}
            onChange={(event) => saveSetting("vaultPath", event.target.value)}
            onBlur={() => refresh().catch((error) => setMessage(String(error)))}
          />
        </Field>
        <div className="rounded-md border bg-muted/30 p-3">
          <p className="break-all font-mono text-xs text-muted-foreground">
            {status?.path ?? "Loading…"}
          </p>
          <p className="mt-2 text-xs text-muted-foreground">
            {status?.generatedFiles ?? 0} generated Markdown files · {status?.importedNotes ?? 0} imported notes
          </p>
        </div>
        {message ? <FieldDescription role="status">{message}</FieldDescription> : null}
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" disabled={busy} onClick={sync}>
            <RefreshCwIcon className={busy ? "animate-spin" : ""} />
            {busy ? "Syncing…" : "Sync now"}
          </Button>
          <Button variant="outline" size="sm" onClick={() => api.vaultOpen(false).catch((error) => setMessage(String(error)))}>
            <FolderOpenIcon />
            Reveal in Finder
          </Button>
          <Button variant="outline" size="sm" onClick={() => api.vaultOpen(true).catch((error) => setMessage(String(error)))}>
            <BookOpenIcon />
            Open in Obsidian
          </Button>
        </div>
      </FieldGroup>
    </FieldSet>
  );
}
