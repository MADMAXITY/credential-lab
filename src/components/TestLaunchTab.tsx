import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/tauri";

interface SavedCredential {
  id: number;
  launcher: string;
  username: string;
  synced_at: string;
}

interface SwitchResult {
  success: boolean;
  launcher: string;
  new_user: string | null;
  steps: string[];
  error: string | null;
}

interface LauncherState {
  launcher: string;
  is_running: boolean;
  current_user: string | null;
  active_user_id: number;
}

interface TestAccountResult {
  username: string;
  success: boolean;
  message: string;
}

interface TestAllResult {
  results: TestAccountResult[];
  passed: number;
  failed: number;
}

interface Props {
  addLog: (level: string, message: string) => void;
}

export default function TestLaunchTab({ addLog }: Props) {
  const [credentials, setCredentials] = useState<SavedCredential[]>([]);
  const [selectedCred, setSelectedCred] = useState<number | null>(null);
  const [switching, setSwitching] = useState(false);
  const [switchResult, setSwitchResult] = useState<SwitchResult | null>(null);
  const [launcherState, setLauncherState] = useState<LauncherState | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestAllResult | null>(null);

  useEffect(() => {
    loadCredentials();
  }, []);

  const loadCredentials = async () => {
    try {
      const result = await invoke<SavedCredential[]>("list_credentials", { launcherId: null });
      setCredentials(result);
    } catch (e) {
      addLog("error", `Failed to load credentials: ${e}`);
    }
  };

  const switchAccount = async () => {
    if (!selectedCred) return;
    const cred = credentials.find((c) => c.id === selectedCred);
    if (!cred) return;

    setSwitching(true);
    setSwitchResult(null);
    addLog("info", `Switching ${cred.launcher} to: ${cred.username}...`);

    try {
      const result = await invoke<SwitchResult>("switch_account", { credentialId: selectedCred });
      setSwitchResult(result);
      result.steps.forEach((step) => addLog("info", `  ${step}`));
      if (result.success) {
        addLog("info", `Switch successful: now logged in as ${result.new_user}`);
      } else {
        addLog("error", `Switch failed: ${result.error}`);
      }
    } catch (e) {
      addLog("error", `Switch error: ${e}`);
      setSwitchResult({ success: false, launcher: cred.launcher, new_user: null, steps: [], error: String(e) });
    }
    setSwitching(false);
  };

  const testAllAccounts = async (launcherId: string) => {
    setTesting(true);
    setTestResult(null);
    addLog("info", `Testing all saved ${launcherId} accounts...`);

    try {
      const result = await invoke<TestAllResult>("test_all_accounts", { launcherId });
      setTestResult(result);
      result.results.forEach((r) => {
        if (r.success) {
          addLog("info", `  ✓ ${r.username}: ${r.message}`);
        } else {
          addLog("error", `  ✗ ${r.username}: ${r.message}`);
        }
      });
      addLog("info", `Test complete: ${result.passed} passed, ${result.failed} failed`);
    } catch (e) {
      addLog("error", `Test failed: ${e}`);
    }
    setTesting(false);
  };

  const verifyState = async (launcherId: string) => {
    setVerifying(true);
    addLog("info", `Verifying ${launcherId} state...`);
    try {
      const state = await invoke<LauncherState>("verify_launcher_state", { launcherId });
      setLauncherState(state);
      addLog("info", `${launcherId}: ${state.is_running ? "running" : "not running"}, user: ${state.current_user || "none"}, ActiveUser: ${state.active_user_id}`);
    } catch (e) {
      addLog("error", `Verify failed: ${e}`);
    }
    setVerifying(false);
  };

  const launchers = [...new Set(credentials.map((c) => c.launcher))];

  return (
    <div className="space-y-6 max-w-2xl">
      <h2 className="text-lg font-semibold">Test Account Switching</h2>

      {/* Step 1: Switch to a specific account */}
      <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)] p-4 space-y-3">
        <div className="flex items-center gap-2">
          <span className="w-6 h-6 rounded-full bg-[var(--accent)]/15 text-[var(--accent)] text-xs font-bold flex items-center justify-center">1</span>
          <h3 className="font-medium">Switch Account</h3>
        </div>

        {credentials.length === 0 ? (
          <p className="text-sm text-[var(--text-muted)] italic ml-8">
            No saved credentials. Go to the Credentials tab and sync some accounts first.
          </p>
        ) : (
          <div className="ml-8 space-y-3">
            <select
              value={selectedCred || ""}
              onChange={(e) => setSelectedCred(Number(e.target.value) || null)}
              className="w-full px-3 py-2 rounded-lg bg-[var(--bg-primary)] border border-[var(--border)] text-sm text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
            >
              <option value="">Choose a saved credential...</option>
              {launchers.map((launcher) => (
                <optgroup key={launcher} label={launcher.charAt(0).toUpperCase() + launcher.slice(1)}>
                  {credentials
                    .filter((c) => c.launcher === launcher)
                    .map((cred) => (
                      <option key={cred.id} value={cred.id}>
                        {cred.username}
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>

            <button
              onClick={switchAccount}
              disabled={!selectedCred || switching}
              className="px-5 py-2.5 rounded-lg bg-[var(--accent)] text-black font-semibold text-sm hover:brightness-110 transition disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {switching ? "Switching..." : "Switch Account"}
            </button>
          </div>
        )}
      </div>

      {/* Switch result */}
      {switchResult && (
        <div
          className={`rounded-xl border p-4 space-y-2 ${
            switchResult.success
              ? "border-[var(--accent)]/30 bg-[var(--accent)]/5"
              : "border-[var(--danger)]/30 bg-[var(--danger)]/5"
          }`}
        >
          <h3 className="font-medium">
            {switchResult.success ? "Switch Successful" : "Switch Failed"}
          </h3>
          <div className="space-y-1">
            {switchResult.steps.map((step, i) => (
              <p key={i} className="text-sm text-[var(--text-secondary)]">{step}</p>
            ))}
            {switchResult.error && (
              <p className="text-sm text-[var(--danger)]">{switchResult.error}</p>
            )}
          </div>
        </div>
      )}

      {/* Step 2: Test All */}
      <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)] p-4 space-y-3">
        <div className="flex items-center gap-2">
          <span className="w-6 h-6 rounded-full bg-[var(--accent)]/15 text-[var(--accent)] text-xs font-bold flex items-center justify-center">2</span>
          <h3 className="font-medium">Test All Accounts</h3>
          <span className="text-xs text-[var(--text-muted)]">
            Switches to each saved account, verifies auto-login works
          </span>
        </div>

        <div className="ml-8 flex gap-2">
          {launchers.length > 0 ? (
            launchers.map((launcher) => (
              <button
                key={launcher}
                onClick={() => testAllAccounts(launcher)}
                disabled={testing}
                className="px-4 py-2 text-sm font-medium rounded-lg bg-[var(--warning)]/10 text-[var(--warning)] border border-[var(--warning)]/30 hover:bg-[var(--warning)]/20 transition-colors disabled:opacity-50"
              >
                {testing ? "Testing..." : `Test ${launcher.charAt(0).toUpperCase() + launcher.slice(1)} Accounts`}
              </button>
            ))
          ) : (
            <p className="text-sm text-[var(--text-muted)] italic">No saved credentials to test.</p>
          )}
        </div>

        {testResult && (
          <div className="ml-8 mt-3 space-y-2">
            <p className="text-sm font-medium">
              <span className="text-[var(--accent)]">{testResult.passed} passed</span>
              {testResult.failed > 0 && (
                <span className="text-[var(--danger)]"> · {testResult.failed} failed</span>
              )}
            </p>
            {testResult.results.map((r, i) => (
              <div key={i} className={`flex items-center gap-2 text-sm px-3 py-2 rounded-lg ${
                r.success ? "bg-[var(--accent)]/5" : "bg-[var(--danger)]/5"
              }`}>
                <span>{r.success ? "✓" : "✗"}</span>
                <span className="font-medium">{r.username}</span>
                <span className="text-[var(--text-muted)]">— {r.message}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Step 3: Verify state */}
      <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)] p-4 space-y-3">
        <div className="flex items-center gap-2">
          <span className="w-6 h-6 rounded-full bg-[var(--accent)]/15 text-[var(--accent)] text-xs font-bold flex items-center justify-center">3</span>
          <h3 className="font-medium">Verify State</h3>
        </div>

        <div className="ml-8 flex gap-2">
          {["steam", "epic", "riot"].map((id) => (
            <button
              key={id}
              onClick={() => verifyState(id)}
              disabled={verifying}
              className="px-3 py-1.5 text-xs font-medium rounded-lg bg-[var(--bg-primary)] border border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:border-[var(--accent)] transition-colors"
            >
              Check {id.charAt(0).toUpperCase() + id.slice(1)}
            </button>
          ))}
        </div>

        {launcherState && (
          <div className="ml-8 p-3 rounded-lg bg-[var(--bg-primary)] text-sm space-y-1">
            <p><span className="text-[var(--text-muted)]">Launcher:</span> <span className="font-medium">{launcherState.launcher}</span></p>
            <p><span className="text-[var(--text-muted)]">Running:</span> <span className={launcherState.is_running ? "text-[var(--accent)]" : "text-[var(--text-muted)]"}>{launcherState.is_running ? "Yes" : "No"}</span></p>
            <p><span className="text-[var(--text-muted)]">Current User:</span> <span className="font-medium">{launcherState.current_user || "(none)"}</span></p>
            <p><span className="text-[var(--text-muted)]">ActiveUser ID:</span> <span className="font-medium">{launcherState.active_user_id || "0 (login screen)"}</span></p>
          </div>
        )}
      </div>
    </div>
  );
}
