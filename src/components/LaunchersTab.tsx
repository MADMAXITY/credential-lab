import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/tauri";

interface LauncherInfo {
  id: string;
  name: string;
  is_installed: boolean;
  install_path: string | null;
  current_user: string | null;
  is_running: boolean;
  remembered_accounts: string[];
}

interface DetectedGame {
  game_id: string;
  name: string;
  launcher: string;
  install_path: string | null;
}

interface Props {
  addLog: (level: string, message: string) => void;
}

export default function LaunchersTab({ addLog }: Props) {
  const [launchers, setLaunchers] = useState<LauncherInfo[]>([]);
  const [games, setGames] = useState<Record<string, DetectedGame[]>>({});
  const [expandedLauncher, setExpandedLauncher] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const detectLaunchers = async () => {
    setLoading(true);
    addLog("info", "Detecting installed launchers...");
    try {
      const result = await invoke<LauncherInfo[]>("detect_launchers");
      setLaunchers(result);
      const installed = result.filter((l) => l.is_installed);
      addLog("info", `Found ${installed.length} installed launcher(s)`);
      installed.forEach((l) => {
        addLog("info", `${l.name}: ${l.current_user ? `logged in as ${l.current_user}` : "no user"} ${l.is_running ? "(running)" : ""}`);
        if (l.remembered_accounts.length > 0) {
          addLog("info", `  Remembered accounts: ${l.remembered_accounts.join(", ")}`);
        }
      });
    } catch (e) {
      addLog("error", `Detection failed: ${e}`);
    }
    setLoading(false);
  };

  const detectGames = async (launcherId: string) => {
    addLog("info", `Scanning games for ${launcherId}...`);
    try {
      const result = await invoke<DetectedGame[]>("detect_games", { launcherId });
      setGames((prev) => ({ ...prev, [launcherId]: result }));
      addLog("info", `Found ${result.length} game(s) for ${launcherId}`);
    } catch (e) {
      addLog("error", `Game scan failed: ${e}`);
    }
  };

  const toggleLauncher = (id: string) => {
    if (expandedLauncher === id) {
      setExpandedLauncher(null);
    } else {
      setExpandedLauncher(id);
      if (!games[id]) {
        detectGames(id);
      }
    }
  };

  useEffect(() => {
    detectLaunchers();
  }, []);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Installed Launchers</h2>
        <button
          onClick={detectLaunchers}
          disabled={loading}
          className="px-4 py-2 text-sm font-medium rounded-lg bg-[var(--bg-card)] border border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:border-[var(--accent)] transition-colors disabled:opacity-50"
        >
          {loading ? "Scanning..." : "Refresh"}
        </button>
      </div>

      <div className="space-y-2">
        {launchers.map((launcher) => (
          <div
            key={launcher.id}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden"
          >
            {/* Launcher header */}
            <button
              onClick={() => launcher.is_installed && toggleLauncher(launcher.id)}
              className="w-full flex items-center gap-4 px-4 py-3 hover:bg-[var(--bg-hover)] transition-colors text-left"
            >
              {/* Status dot */}
              <div
                className={`w-2.5 h-2.5 rounded-full shrink-0 ${
                  launcher.is_installed
                    ? launcher.is_running
                      ? "bg-[var(--accent)]"
                      : "bg-[var(--warning)]"
                    : "bg-[var(--text-muted)]"
                }`}
              />

              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-semibold text-[var(--text-primary)]">{launcher.name}</span>
                  {launcher.is_running && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--accent)]/10 text-[var(--accent)] font-medium">
                      RUNNING
                    </span>
                  )}
                </div>
                <div className="text-xs text-[var(--text-muted)] mt-0.5">
                  {launcher.is_installed
                    ? launcher.current_user
                      ? `User: ${launcher.current_user}`
                      : "No user logged in"
                    : "Not installed"}
                </div>
              </div>

              {/* Remembered accounts count */}
              {launcher.remembered_accounts.length > 0 && (
                <span className="text-xs text-[var(--text-muted)] px-2 py-1 rounded-md bg-[var(--bg-primary)]">
                  {launcher.remembered_accounts.length} saved
                </span>
              )}

              {/* Expand arrow */}
              {launcher.is_installed && (
                <svg
                  className={`w-4 h-4 text-[var(--text-muted)] transition-transform ${
                    expandedLauncher === launcher.id ? "rotate-180" : ""
                  }`}
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
              )}
            </button>

            {/* Expanded: remembered accounts + games */}
            {expandedLauncher === launcher.id && (
              <div className="border-t border-[var(--border)] px-4 py-3 space-y-3">
                {/* Remembered accounts */}
                {launcher.remembered_accounts.length > 0 && (
                  <div>
                    <h4 className="text-xs font-semibold text-[var(--text-muted)] uppercase tracking-wider mb-2">
                      Remembered Accounts
                    </h4>
                    <div className="flex flex-wrap gap-2">
                      {launcher.remembered_accounts.map((acc) => (
                        <span
                          key={acc}
                          className={`text-xs px-2.5 py-1 rounded-md ${
                            acc === launcher.current_user
                              ? "bg-[var(--accent)]/15 text-[var(--accent)] border border-[var(--accent)]/30"
                              : "bg-[var(--bg-primary)] text-[var(--text-secondary)] border border-[var(--border)]"
                          }`}
                        >
                          {acc}
                          {acc === launcher.current_user && " (active)"}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Games */}
                <div>
                  <h4 className="text-xs font-semibold text-[var(--text-muted)] uppercase tracking-wider mb-2">
                    Installed Games ({games[launcher.id]?.length ?? "..."})
                  </h4>
                  {games[launcher.id] ? (
                    <div className="grid grid-cols-2 gap-1.5 max-h-48 overflow-y-auto">
                      {games[launcher.id].map((game) => (
                        <div
                          key={game.game_id}
                          className="text-xs px-2.5 py-1.5 rounded bg-[var(--bg-primary)] text-[var(--text-secondary)] truncate"
                          title={game.install_path || undefined}
                        >
                          {game.name}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="text-xs text-[var(--text-muted)] italic">Loading...</p>
                  )}
                </div>

                {/* Install path */}
                {launcher.install_path && (
                  <p className="text-xs text-[var(--text-muted)] mt-2 truncate" title={launcher.install_path}>
                    Path: {launcher.install_path}
                  </p>
                )}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
