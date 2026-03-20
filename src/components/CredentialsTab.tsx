import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/tauri";

interface SavedCredential {
  id: number;
  launcher: string;
  username: string;
  synced_at: string;
  file_count: number;
  total_size: number;
  notes: string;
}

interface Props {
  addLog: (level: string, message: string) => void;
}

const LAUNCHERS = [
  { id: "steam", name: "Steam" },
  { id: "epic", name: "Epic Games" },
  { id: "riot", name: "Riot Games" },
  { id: "ea", name: "EA App" },
  { id: "ubisoft", name: "Ubisoft Connect" },
  { id: "gog", name: "GOG Galaxy" },
];

export default function CredentialsTab({ addLog }: Props) {
  const [credentials, setCredentials] = useState<SavedCredential[]>([]);
  const [syncing, setSyncing] = useState<string | null>(null);

  const loadCredentials = async () => {
    try {
      const result = await invoke<SavedCredential[]>("list_credentials", { launcherId: null });
      setCredentials(result);
    } catch (e) {
      addLog("error", `Failed to load credentials: ${e}`);
    }
  };

  const syncCredential = async (launcherId: string) => {
    setSyncing(launcherId);
    addLog("info", `Syncing current ${launcherId} account...`);
    try {
      const result = await invoke<{ success: boolean; username: string; message: string }>(
        "sync_current_credential",
        { launcherId }
      );
      if (result.success) {
        addLog("info", `Synced ${launcherId}: ${result.username}`);
        loadCredentials();
      }
    } catch (e) {
      addLog("error", `Sync failed: ${e}`);
    }
    setSyncing(null);
  };

  const removeCredential = async (id: number, launcher: string, username: string) => {
    addLog("info", `Removing ${launcher} credential: ${username}`);
    try {
      await invoke("remove_credential", { credentialId: id });
      addLog("info", `Removed ${username}`);
      loadCredentials();
    } catch (e) {
      addLog("error", `Failed to remove: ${e}`);
    }
  };

  useEffect(() => {
    loadCredentials();
  }, []);

  const timeAgo = (isoDate: string) => {
    try {
      const date = new Date(isoDate + "Z");
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffMins = Math.floor(diffMs / 60000);
      if (diffMins < 1) return "just now";
      if (diffMins < 60) return `${diffMins}m ago`;
      const diffHours = Math.floor(diffMins / 60);
      if (diffHours < 24) return `${diffHours}h ago`;
      const diffDays = Math.floor(diffHours / 24);
      return `${diffDays}d ago`;
    } catch {
      return isoDate;
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Saved Credentials</h2>
        <p className="text-xs text-[var(--text-muted)]">
          Log into each account in the launcher with "Remember me" → then Sync Current
        </p>
      </div>

      {LAUNCHERS.map((launcher) => {
        const launcherCreds = credentials.filter((c) => c.launcher === launcher.id);

        return (
          <div key={launcher.id} className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden">
            {/* Launcher header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--border)]">
              <div className="flex items-center gap-3">
                <span className="font-semibold text-[var(--text-primary)]">{launcher.name}</span>
                <span className="text-xs text-[var(--text-muted)]">
                  {launcherCreds.length} saved
                </span>
              </div>
              <button
                onClick={() => syncCredential(launcher.id)}
                disabled={syncing === launcher.id}
                className="px-3 py-1.5 text-xs font-medium rounded-lg bg-[var(--accent)]/10 text-[var(--accent)] border border-[var(--accent)]/30 hover:bg-[var(--accent)]/20 transition-colors disabled:opacity-50"
              >
                {syncing === launcher.id ? "Syncing..." : "Sync Current"}
              </button>
            </div>

            {/* Credentials list */}
            <div className="divide-y divide-[var(--border)]">
              {launcherCreds.length === 0 ? (
                <p className="px-4 py-4 text-sm text-[var(--text-muted)] italic">
                  No credentials saved. Log into {launcher.name} with "Remember me" and click "Sync Current".
                </p>
              ) : (
                launcherCreds.map((cred) => (
                  <div
                    key={cred.id}
                    className="flex items-center justify-between px-4 py-3 hover:bg-[var(--bg-hover)] transition-colors"
                  >
                    <div className="flex items-center gap-3 min-w-0">
                      <div className="w-8 h-8 rounded-lg bg-[var(--accent)]/10 flex items-center justify-center shrink-0">
                        <span className="text-[var(--accent)] text-xs font-bold">
                          {cred.username[0]?.toUpperCase()}
                        </span>
                      </div>
                      <div className="min-w-0">
                        <p className="text-sm font-medium text-[var(--text-primary)] truncate">
                          {cred.username}
                        </p>
                        <p className="text-xs text-[var(--text-muted)]">
                          Synced {timeAgo(cred.synced_at)}
                        </p>
                      </div>
                    </div>

                    <button
                      onClick={() => removeCredential(cred.id, cred.launcher, cred.username)}
                      className="p-2 rounded-lg text-[var(--text-muted)] hover:text-[var(--danger)] hover:bg-[var(--danger)]/10 transition-colors shrink-0"
                      title="Remove credential"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                      </svg>
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
