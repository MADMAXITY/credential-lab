# Credential Lab

**Standalone dev tool for testing game launcher credential management.**

Part of the [ArcadeOS](https://arcadeosgaming.com) platform — built to validate account switching mechanics before integrating into the PC Client.

---

## What It Does

Credential Lab lets you **sync, store, and switch between multiple game launcher accounts** on a single PC. It captures the launcher's auth state when you sync an account, then restores it when you switch — enabling instant account switching without re-entering passwords.

### Supported Launchers

| Launcher | Sync | Switch | Status |
|----------|------|--------|--------|
| Steam | Yes | Yes | **Working** |
| Epic Games | Yes | Yes | **Working** |
| Riot Games | - | - | Not possible (Vanguard anti-cheat) |
| EA App | Planned | Planned | - |
| Ubisoft Connect | Planned | Planned | - |
| GOG Galaxy | Planned | Planned | - |

---

## How It Works

### The Problem
Gaming cafes need multiple Steam/Epic accounts (for paid games like GTA V, FIFA, Forza). Switching between accounts manually means logging out, logging in, entering passwords, waiting for 2FA — slow and exposes credentials.

### The Solution
```
1. Log into Account A in Steam (with "Remember me") -> Click "Sync Current"
   -> Saves config.vdf + loginusers.vdf (contains auth tokens)

2. Log out, log into Account B -> Click "Sync Current"
   -> Saves Account B's auth state separately

3. Click "Switch to Account A"
   -> Kills Steam
   -> Restores Account A's saved config.vdf (with its auth tokens)
   -> Patches loginusers.vdf (MostRecent=1)
   -> Sets registry (AutoLoginUser)
   -> Restarts Steam
   -> Steam auto-logs into Account A — no password needed
```

### What Gets Saved Per Account

**Steam:**
- `config/config.vdf` — contains `Authentication > RememberedMachineID` JWT tokens
- `config/loginusers.vdf` — account entries with `RememberPassword` flags
- Registry: `HKCU\Software\Valve\Steam\AutoLoginUser` + `RememberPassword`

**Epic Games:**
- `Saved/Config/` directory — `GameUserSettings.ini` + all config files
- Registry: `HKCU\Software\Epic Games\Unreal Engine\Identifiers\AccountId`
- Clears EOS cache directories on switch

---

## Screenshots

### Launchers Tab
Detects installed launchers, shows current logged-in user, lists remembered accounts and installed games.

### Credentials Tab
Sync the currently logged-in account. View and manage all saved credentials.

### Test Launch Tab
Switch between saved accounts with real-time step-by-step logging. Test All verifies every saved account works.

---

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Backend | **Rust** (Tauri 1.5) |
| Frontend | **React** + TypeScript + Vite |
| Styling | **Tailwind CSS** v4 |
| Storage | **SQLite** (rusqlite) |
| Steam | **steamlocate** crate + **winreg** |
| Encryption | DPAPI (planned) |

---

## Getting Started

### Prerequisites
- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) (18+)
- Windows 10/11 (launcher APIs are Windows-only)
- Steam and/or Epic Games installed

### Run
```bash
git clone https://github.com/MADMAXITY/credential-lab.git
cd credential-lab
npm install
npm run tauri dev
```

First build takes a few minutes (compiling Tauri + Rust crates). Subsequent builds are fast.

### Build
```bash
npm run tauri build
```

---

## Project Structure

```
credential-lab/
├── src-tauri/src/
│   ├── main.rs              # Tauri app setup + command registration
│   ├── db.rs                # SQLite credential storage + operation log
│   ├── encryption.rs        # DPAPI placeholder (TODO)
│   ├── launcher_detect.rs   # Detect Steam, Epic, Riot, EA, Ubisoft, GOG
│   ├── game_detect.rs       # Scan installed games per launcher
│   ├── switcher.rs          # Account switching (kill → restore → restart → verify)
│   └── credentials/
│       ├── mod.rs            # Sync/list/remove commands
│       ├── steam.rs          # Steam: sync config.vdf + loginusers.vdf, restore on switch
│       └── epic.rs           # Epic: sync Config\ folder + registry, restore on switch
├── src/
│   ├── App.tsx               # Tab layout (Launchers, Credentials, Test Launch)
│   ├── components/
│   │   ├── LaunchersTab.tsx   # Launcher detection + game scanning
│   │   ├── CredentialsTab.tsx # Sync current + manage saved credentials
│   │   ├── TestLaunchTab.tsx  # Switch accounts + test all + verify state
│   │   └── LogPanel.tsx       # Real-time operation log
│   └── styles/
│       └── globals.css        # Dark theme
└── package.json
```

---

## Key Discoveries

1. **Steam's newer auth uses JWT tokens** in `config.vdf` under `Authentication > RememberedMachineID`. The old VDF + registry approach alone is insufficient — `config.vdf` must be saved and restored per-account.

2. **`loginusers.vdf` `RememberPassword=1` doesn't mean the token is valid.** Steam sets this flag for every account that ever checked "Remember me", even if the token later expired.

3. **Switching verification** uses `HKCU\Software\Valve\Steam\ActiveProcess\ActiveUser` — value `0` means login screen (failed), non-zero is the logged-in SteamID32.

4. **Riot/Valorant cannot be switched** — Vanguard kernel anti-cheat blocks credential file manipulation.

5. **Each account must be logged in once with "Remember me"** on each PC (pre-seeding). After that, switching is instant via file restore.

---

## Relationship to ArcadeOS

This is a standalone dev tool. The validated switching mechanics will be integrated into the ArcadeOS PC Client as part of the **Credential Pooling** feature:

1. **Backend** — Pool model + account checkout API
2. **Admin Dashboard** — Pool management UI
3. **PC Client** — On game launch: checkout account from pool → switch credentials → launch game → on exit: release account

See the [Credential Pooling design doc](../Docs/plans/credential-lab-plan.md) for the full integration plan.

---

## License

Internal tool — part of ArcadeOS platform.
