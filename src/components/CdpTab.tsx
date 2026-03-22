import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/tauri";

interface AutoLoginResult {
  success: boolean;
  launcher: string;
  steps: string[];
  error: string | null;
}

interface LoginAccount {
  id: number;
  launcher: string;
  label: string;
  username: string;
  password: string;
  created_at: string;
}

interface Props {
  addLog: (level: string, message: string) => void;
}

export default function CdpTab({ addLog }: Props) {
  const [loading, setLoading] = useState(false);
  const [activeAccountId, setActiveAccountId] = useState<number | null>(null);
  const [steps, setSteps] = useState<string[]>([]);
  const [accounts, setAccounts] = useState<LoginAccount[]>([]);
  const [selectedLauncher, setSelectedLauncher] = useState<"epic" | "ea">("epic");

  // Add account form
  const [showForm, setShowForm] = useState(false);
  const [formLabel, setFormLabel] = useState("");
  const [formUsername, setFormUsername] = useState("");
  const [formPassword, setFormPassword] = useState("");

  const loadAccounts = async () => {
    try {
      const res = await invoke<LoginAccount[]>("list_login_accounts", {
        launcherId: selectedLauncher,
      });
      setAccounts(res);
    } catch (e: any) {
      addLog("error", `Failed to load accounts: ${e}`);
    }
  };

  useEffect(() => {
    loadAccounts();
  }, [selectedLauncher]);

  const saveAccount = async () => {
    if (!formLabel || !formUsername || !formPassword) return;
    try {
      await invoke("save_login_account", {
        launcherId: selectedLauncher,
        label: formLabel,
        username: formUsername,
        password: formPassword,
      });
      addLog("info", `Saved account: ${formLabel}`);
      setShowForm(false);
      setFormLabel("");
      setFormUsername("");
      setFormPassword("");
      loadAccounts();
    } catch (e: any) {
      addLog("error", `Save failed: ${e}`);
    }
  };

  const removeAccount = async (id: number, label: string) => {
    try {
      await invoke("remove_login_account", { accountId: id });
      addLog("info", `Removed account: ${label}`);
      loadAccounts();
    } catch (e: any) {
      addLog("error", `Remove failed: ${e}`);
    }
  };

  const loginWithAccount = async (account: LoginAccount) => {
    setLoading(true);
    setActiveAccountId(account.id);
    setSteps([]);
    addLog("info", `Auto-login: ${account.label}...`);
    try {
      const res = await invoke<AutoLoginResult>("autologin_with_account", {
        accountId: account.id,
      });
      setSteps(res.steps);
      addLog(
        res.success ? "info" : "warn",
        `Auto-login ${res.success ? "succeeded" : "needs review"} for ${account.label}`
      );
    } catch (e: any) {
      addLog("error", `Auto-login error: ${e}`);
      setSteps((prev) => [...prev, `Error: ${e}`]);
    } finally {
      setLoading(false);
      setActiveAccountId(null);
    }
  };

  const quickLogin = async () => {
    if (!formUsername || !formPassword) {
      addLog("error", "Username and password required");
      return;
    }
    setLoading(true);
    setSteps([]);
    addLog("info", `Quick login for ${selectedLauncher}...`);
    try {
      const res = await invoke<AutoLoginResult>("cdp_login", {
        launcherId: selectedLauncher,
        username: formUsername,
        password: formPassword,
      });
      setSteps(res.steps);
      addLog(
        res.success ? "info" : "warn",
        `Quick login ${res.success ? "succeeded" : "needs review"}`
      );
    } catch (e: any) {
      addLog("error", `Quick login error: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-5">
      <div>
        <h2 className="text-base font-semibold text-[var(--text-primary)] mb-1">
          Auto-Login (SendInput)
        </h2>
        <p className="text-xs text-[var(--text-muted)]">
          Save accounts, click to auto-login. Uses SendInput to type credentials
          into the launcher. No coordinates config needed.
        </p>
      </div>

      {/* Launcher selection */}
      <div className="flex gap-2">
        {(["epic", "ea"] as const).map((id) => (
          <button
            key={id}
            onClick={() => setSelectedLauncher(id)}
            className={`px-4 py-2 text-sm rounded-md border transition-colors ${
              selectedLauncher === id
                ? "border-[var(--accent)] text-[var(--accent)] bg-[var(--accent-bg)]"
                : "border-[var(--border)] text-[var(--text-secondary)] hover:border-[var(--text-muted)]"
            }`}
          >
            {id === "epic" ? "Epic Games" : "EA Desktop"}
          </button>
        ))}
      </div>

      {/* Saved accounts */}
      <div className="p-4 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)]">
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-medium text-[var(--text-primary)]">
            Saved Accounts ({accounts.length})
          </h3>
          <button
            onClick={() => setShowForm(!showForm)}
            className="px-3 py-1.5 text-xs bg-[var(--accent)] text-white rounded-md hover:opacity-90"
          >
            {showForm ? "Cancel" : "+ Add Account"}
          </button>
        </div>

        {/* Add account form */}
        {showForm && (
          <div className="mb-4 p-3 rounded-md border border-[var(--border)] bg-[var(--bg-primary)] space-y-2">
            <input
              type="text"
              placeholder="Label (e.g. Pool Account 1)"
              value={formLabel}
              onChange={(e) => setFormLabel(e.target.value)}
              className="w-full px-3 py-2 text-sm bg-[var(--bg-secondary)] border border-[var(--border)] rounded-md text-[var(--text-primary)] placeholder:text-[var(--text-muted)]"
            />
            <input
              type="email"
              placeholder="Email / Username"
              value={formUsername}
              onChange={(e) => setFormUsername(e.target.value)}
              className="w-full px-3 py-2 text-sm bg-[var(--bg-secondary)] border border-[var(--border)] rounded-md text-[var(--text-primary)] placeholder:text-[var(--text-muted)]"
            />
            <input
              type="password"
              placeholder="Password"
              value={formPassword}
              onChange={(e) => setFormPassword(e.target.value)}
              className="w-full px-3 py-2 text-sm bg-[var(--bg-secondary)] border border-[var(--border)] rounded-md text-[var(--text-primary)] placeholder:text-[var(--text-muted)]"
            />
            <div className="flex gap-2">
              <button
                onClick={saveAccount}
                disabled={!formLabel || !formUsername || !formPassword}
                className="px-3 py-1.5 text-xs bg-green-600 text-white rounded-md hover:bg-green-700 disabled:opacity-50"
              >
                Save Account
              </button>
              <button
                onClick={quickLogin}
                disabled={loading || !formUsername || !formPassword}
                className="px-3 py-1.5 text-xs bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50"
              >
                {loading ? "Logging in..." : "Quick Login (don't save)"}
              </button>
            </div>
          </div>
        )}

        {/* Account list */}
        {accounts.length === 0 && !showForm && (
          <p className="text-xs text-[var(--text-muted)] py-4 text-center">
            No accounts saved. Click "+ Add Account" to add one.
          </p>
        )}

        <div className="space-y-2">
          {accounts.map((acc) => (
            <div
              key={acc.id}
              className="flex items-center justify-between p-3 rounded-md border border-[var(--border)] bg-[var(--bg-primary)] hover:border-[var(--text-muted)] transition-colors"
            >
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  {acc.label}
                </div>
                <div className="text-xs text-[var(--text-muted)] truncate">
                  {acc.username}
                </div>
              </div>
              <div className="flex gap-2 ml-3 shrink-0">
                <button
                  onClick={() => loginWithAccount(acc)}
                  disabled={loading}
                  className={`px-4 py-1.5 text-xs font-medium rounded-md transition-colors ${
                    loading && activeAccountId === acc.id
                      ? "bg-yellow-600 text-white"
                      : "bg-[var(--accent)] text-white hover:opacity-90"
                  } disabled:opacity-50`}
                >
                  {loading && activeAccountId === acc.id
                    ? "Logging in..."
                    : "Login"}
                </button>
                <button
                  onClick={() => removeAccount(acc.id, acc.label)}
                  disabled={loading}
                  className="px-2 py-1.5 text-xs text-red-400 border border-red-400/30 rounded-md hover:bg-red-400/10 disabled:opacity-50"
                >
                  Remove
                </button>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Steps output */}
      {steps.length > 0 && (
        <div className="p-4 rounded-lg border border-[var(--border)] bg-[var(--bg-primary)]">
          <h3 className="text-sm font-medium text-[var(--text-primary)] mb-2">
            Steps
          </h3>
          <div className="space-y-0.5 max-h-64 overflow-y-auto">
            {steps.map((step, i) => (
              <div
                key={i}
                className={`text-xs font-mono py-0.5 ${
                  step.toLowerCase().includes("error") ||
                  step.toLowerCase().includes("failed")
                    ? "text-red-400"
                    : step.toLowerCase().includes("success") ||
                      step.toLowerCase().includes("typing") ||
                      step.toLowerCase().includes("clicking")
                    ? "text-green-400"
                    : "text-[var(--text-muted)]"
                }`}
              >
                {i + 1}. {step}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
