import type { ManualChapter } from './manualTypes';

export const manualChaptersEn: ManualChapter[] = [
  {
    id: 'terminal',
    title: 'Terminal & Shell',
    icon: 'Terminal',
    sections: [
      {
        id: 'terminal-pty',
        title: 'Local Terminal & System PTY',
        badge: 'PTY',
        keywords: ['cmd', 'zsh', 'bash', 'cd', 'drive switch', 'local terminal', 'adaptation'],
        content: `### Overview
VeloTerm natively integrates system-level pseudoterminals (PTY). On macOS and Linux, it loads the user login shell directly (zsh/bash/fish); on Windows, it boots native \`cmd.exe\` injected with a command adaptation layer, standardizing cross-platform terminal workflows.

### Entry & Shortcuts
- **New Local Terminal**: Click the "+" button in the top header, or press \`⌘N\` (macOS) / \`Ctrl+N\` (Windows).
- **Welcome Screen**: Click "New Local Terminal" from the welcome screen cards.

### Core Steps
1. Press \`⌘N\` / \`Ctrl+N\` to open a terminal; it lands in your current user home directory.
2. **Windows Smart Drive Switching**: In Windows cmd, type \`cd D:\\folder\` or \`chdir E:\\project\` directly; the underlying doskey macro automatically injects the \`/d\` flag without manual typing.
3. **Quick Listing**: Native POSIX \`ll\` alias is mapped to \`ls -lh\`.

### Tips & Notes
- On Windows, the adaptation layer only applies to interactive command lines without altering registry keys or global environment variables.
- Local and remote sessions share the same rendering engine, supporting full ANSI colors, 24-bit TrueColor, and Unicode/Emoji glyphs.`,
      },
      {
        id: 'terminal-ssh',
        title: 'SSH Sessions & Multi-Factor Auth',
        badge: 'SSH',
        keywords: ['SSH', 'Private Key', 'Password', 'ProxyJump', 'Agent', 'Passwd', 'ZMODEM', 'rz', 'sz'],
        content: `### Overview
A production-grade SSH client powered by an asynchronous Rust stack. Supports multiple authentication mechanisms, multi-hop jump hosts (ProxyJump), automated password-expiry state machine, ZMODEM file transfers, and auto-reconnection.

### Entry & Shortcuts
- **Connect Host**: In the "Hosts" list on the sidebar, double-click any host card or click its "Terminal" icon.
- **Reconnect Session**: Click the "Reconnect" icon in the top toolbar or press \`⌘R\` (macOS) / \`Ctrl+R\` (Windows).

### Core Steps
1. **Authentication**: Choose between password, OpenSSH/PEM private keys (with passphrase caching), and system SSH Agent.
2. **Multi-Hop Proxy**: Configure jump host chains to seamlessly penetrate multi-tier isolated bastions.
3. **Password Expiry**: If a remote account requires an immediate password change upon login, the state machine captures the remote prompts, securely updates the credentials, and saves them locally.
4. **ZMODEM Transfers**: Run \`sz <filename>\` to send or \`rz\` to receive files; the client automatically pops up a transfer status overlay.

### Tips & Notes
- Reconnecting preserves full scrollback history and clearly stamps a \`-- Reconnected --\` divider line.
- First-time host connections trigger a Known Hosts fingerprint prompt, safely committing verified keys to the encrypted store.`,
      },
      {
        id: 'terminal-layout',
        title: 'Tabs & Grid Layouts',
        badge: 'Layout',
        keywords: ['Tabs', 'Split', 'Grid', 'Tiling', 'Multi-Session'],
        content: `### Overview
Provides both standard horizontal tab switching (Tabs) and adaptive multi-pane grid tiling (Grid), catering to both focused single-host tasks and multi-node inspection.

### Entry & Shortcuts
- **New Tab**: Click the "+" button in the tab bar or press \`⌘T\` / \`Ctrl+T\`.
- **Close Tab**: Click the "×" on the tab or press \`⌘W\` / \`Ctrl+W\`.
- **Switch Session**: Press \`⌘1\` ~ \`⌘9\` (macOS) / \`Ctrl+1\` ~ \`Ctrl+9\` (Windows) to jump directly to numbered tabs.
- **Toggle Layout**: Click the Grid/Tabs icon on the right side of the terminal header.

### Core Steps
1. Open multiple sessions; tab badges display connection states, environment badges (PROD/STAGE), and recording pills.
2. Drag tab titles to reorder sessions; double-click a tab title to set a temporary alias.
3. Switch to Grid mode to tile all active sessions in an adaptive matrix with independent outputs and scrolling.

### Tips & Notes
- In Grid mode, inputs remain isolated per pane. To type simultaneously across multiple hosts, use the Broadcast Bar.`,
      },
      {
        id: 'terminal-broadcast',
        title: 'Broadcast Console',
        badge: 'Sync',
        keywords: ['Broadcast', 'Multi-Host', 'Sync Input', 'Batch Ops'],
        content: `### Overview
Allows grouping multiple active terminals into a broadcast cohort, sending keystrokes and commands to all grouped sessions simultaneously from a single input bar.

### Entry & Shortcuts
- **Toggle Broadcast Bar**: Click the "Broadcast" icon in the terminal top toolbar.

### Core Steps
1. Click the broadcast icon to expand the bottom input bar.
2. Select which active terminals will join the cohort (support selecting by host or environment).
3. Type commands in the broadcast bar and press Enter; keystrokes are dispatched concurrently.
4. **Emergency Interrupt**: Click "Send Ctrl+C" to instantly broadcast an interrupt signal to kill runaway jobs across all sessions.
5. **Batch Clear**: Click "Clear Screen" to refresh all visible panes simultaneously.

### Tips & Notes
- Verify current working directories and privileges before broadcasting destructive commands, especially in production environments.`,
      },
      {
        id: 'terminal-monitor',
        title: 'Remote Host Lightweight Resource Monitor',
        badge: 'Monitor',
        keywords: ['Resource Monitor', 'CPU', 'Memory', 'Network Rate', 'SWAP', 'Latency', 'Disk'],
        content: `### Overview
Renders real-time remote host metrics at the top of active SSH sessions. Uses an isolated exec sidechannel with zero terminal buffer pollution.

### Entry & Shortcuts
- **Toggle Monitor**: Click the pulse waveform "Resource Monitor" icon in the terminal toolbar (disabled by default).

### Core Steps
1. Click the monitor icon to activate metrics polling for the active SSH session.
2. Samples every 2 seconds: round-trip latency, CPU usage %, physical memory %, SWAP %, network TX/RX throughput, root disk utilization %, and OS distro label.
3. **Responsive Folding**: When terminal panels are narrow, secondary metrics automatically fold via CSS container queries, expanding when space permits.

### Tips & Notes
- Uses precise client-side differential algorithms and only polls the currently active visible tab, suspending in the background to save server resources.`,
      },
      {
        id: 'terminal-recording',
        title: 'Session Recording, Playback & Multi-Format Export',
        badge: 'Record',
        keywords: ['Record', 'Playback', 'Export', 'cast', 'GIF', 'MP4', 'Video', 'Asciinema'],
        content: `### Overview
A clean Asciinema v2 standard sideband recording engine with built-in playback and pure Rust rasterization pipelines, supporting exports to \`.cast\`, \`.log\`, \`.gif\`, and \`.mp4\`.

### Entry & Shortcuts
- **Start Recording**: Right-click inside the terminal and select "Start Recording Session".
- **Pill Control**: A pulsing red REC pill appears at the top-left; click to preview, pause, or stop.
- **Playback & Export**: Right-click to choose "Replay Last Recording", or use the stopped pill menu.

### Core Steps
1. Starting recording tracks only terminal output and resize events; password inputs remain hidden.
2. Stopping displays a 3-second action panel allowing you to:
   - **Play Online**: Opens the built-in modal player with 0.5x~4x speed, scrubber seek, and idle skip (>2s).
   - **Export .cast**: Standard Asciinema JSON Lines file for upload or official CLI play.
   - **Export .log**: Clean plain text log with ANSI escape codes stripped.
   - **Export GIF**: Renders with the built-in \`agg\` Rust engine, matching terminal palettes and system CJK fonts.
   - **Export MP4**: Encodes via built-in FFmpeg \`libgif2mp4\` pipeline into compact H.264 video.

### Tips & Notes
- Fullscreen progress overlays keep you informed during GIF/MP4 encoding before popping the save file dialog.`,
      },
      {
        id: 'terminal-safety',
        title: 'Production Safety Guard & Dangerous Command Interceptor',
        badge: 'Guard',
        keywords: ['Safety Guard', 'Dangerous Command', 'Production', 'rm -rf', 'AST'],
        content: `### Overview
A deep defense mechanism designed to prevent operational disasters. Builds an AST on the backend via \`tree-sitter-bash\` to accurately intercept destructive commands.

### Entry & Shortcuts
- **Setup Environment**: In host settings, mark the environment as \`Staging\` or \`Production\`, and check "Enable Safety Guard".

### Core Steps
1. Production terminals display clear hazard stripes and a red PROD badge on the tab.
2. Pressing Enter (\`\\r\`) or pasting multiline commands in protected sessions pauses execution for syntax tree inspection:
   - Penetrates wrappers such as \`sudo\`, \`nohup\`, \`env\`, and \`bash -c\`.
   - Distinguishes string literals from actual invocations (e.g., \`echo "rm -rf /"\` will not be blocked).
3. **Severity Levels**:
   - \`Critical\` (e.g. \`rm -rf /\`, \`mkfs\`, \`dd\` raw block write, \`chmod 777 /\`): Blocks execution and demands typing a confirmation word (e.g. \`CONFIRM\`).
   - \`High\` (e.g. reboot/shutdown, \`kill -9 1\`, \`redis flushall\`, \`iptables -F\`): Displays a confirmation dialog.
4. Clicking "Cancel" injects \`Ctrl+C\` to reset the command line safely.

### Tips & Notes
- Dev and Test environments bypass this interception to ensure development agility.`,
      },
      {
        id: 'terminal-history',
        title: 'Passive Command History Logging (OSC 7777)',
        badge: 'History',
        keywords: ['Command History', 'OSC 7777', 'Audit', 'Passive Logging'],
        content: `### Overview
A zero-agent passive command logging architecture. Injects a minimal prompt hook into shell profiles to report executed commands and return codes via private OSC 7777 sequences.

### Entry & Shortcuts
- **View History**: Click the "History" icon on the sidebar.
- **Install Hook**: Click "Install Guide" in the History screen to copy the snippet for your shell.

### Core Steps
1. Paste the single-line snippet into your remote \`~/.bashrc\`, \`~/.zshrc\`, or fish config (or configure as a Host Ready Hook).
2. Execute commands normally; when returning to the prompt, the hook automatically reports execution metadata.
3. The client sideband parser strips the private sequence before rendering and commits it to the local encrypted SQLite database.
4. Filter by host, search by text, view failed commands, and rerun or copy records.

### Tips & Notes
- The stripping state machine guarantees hidden escape sequences never corrupt visible terminal outputs even when collection is disabled.`,
      },
      {
        id: 'terminal-interaction',
        title: 'Intelligent Highlighting, URL Links & Drag-and-Drop Escaping',
        badge: 'UX',
        keywords: ['Highlight', 'URL', 'IP', 'Drag Drop', 'Paste', 'Escape', 'Search'],
        content: `### Overview
Polished interactive details: output regex coloring, hoverable clickable URLs/IPs, automatic path escaping on file drag-and-drop, and native clipboard harmony.

### Entry & Shortcuts
- **Find in Terminal**: Focus terminal and press \`⌘F\` (macOS) / \`Ctrl+F\` (Windows).
- **Copy/Paste**: macOS \`⌘C\` / \`⌘V\`; Windows \`Ctrl+Shift+C\` / \`Ctrl+Shift+V\` (with selection, bare \`Ctrl+C\` copies; without selection, bare \`Ctrl+V\` pastes).

### Core Steps
1. **Output Coloring**: Automatically highlights \`ERROR\` and \`FAILED\` in red, \`WARN\` in yellow, \`SUCCESS\` in green, and diff markers.
2. **URL/IP Clicking**: IPv4, IPv6, and HTTP/HTTPS links underline on hover; hold \`⌘\` / \`Ctrl\` and click to open in your default browser.
3. **Drag & Drop**: Dragging files from your desktop into the terminal auto-escapes special characters for POSIX shells or wraps with quotes for Windows cmd.

### Tips & Notes
- The search bar supports regex, case sensitivity, and whole-word matching with Enter / Shift+Enter navigation.`,
      },
      {
        id: 'terminal-ai',
        title: 'AI Terminal Copilot Assistant',
        badge: 'AI',
        keywords: ['AI', 'Copilot', 'DeepSeek', 'Claude', 'OpenAI', 'Gemini', 'Agent'],
        content: `### Overview
A workflow-aware AI side panel supporting OpenAI, DeepSeek, Claude, and Gemini. Reads visible terminal contexts and safely assists with troubleshooting commands.

### Entry & Shortcuts
- **Toggle AI Panel**: Click the "AI" button in the terminal header, or press \`⌘I\` (macOS) / \`Ctrl+I\` (Windows).

### Core Steps
1. Configure API keys and providers under "Settings" -> "AI Configuration" (supports multi-key rotation and custom Base URLs).
2. When encountering errors, press \`⌘I\` to toggle the AI panel on the right.
3. The assistant uses tool loops to inspect visible screens and logs to diagnose root causes.
4. Suggested commands can be sent to the terminal with a single click or executed under step-by-step supervision.

### Tips & Notes
- The panel width is draggable and persisted; it auto-hides when switching away from the terminal page and resumes on return.`,
      },
    ],
  },
  {
    id: 'files',
    title: 'SFTP & File Manager (Files)',
    icon: 'FolderSync',
    sections: [
      {
        id: 'files-manager',
        title: 'Dual-Pane File Manager & Cross-Host Transfers',
        badge: 'SFTP',
        keywords: ['SFTP', 'Dual-Pane', 'Cross-Host', 'Transfer', 'File List', 'Owner'],
        content: `### Overview
A dual-pane file management architecture. Left and right panels can connect independently to local storage or distinct remote SSH hosts, allowing direct drag-and-drop transfers between two remote servers.

### Entry & Shortcuts
- **Open Files**: Click "Files" on the sidebar, or right-click any host card and choose "Open SFTP".

### Core Steps
1. Choose Host A for the left panel and Host B (or Local) for the right panel.
2. Use path input for direct jumps, breadcrumbs for navigation, and instant refresh.
3. Automatically resolves remote numeric uid/gid to human-readable usernames and groups (by parsing remote \`/etc/passwd\` and \`/etc/group\`).
4. **Cross-Host Direct Copy**: Drag files from the left remote panel to the right remote panel; transfers stream point-to-point without saving intermediary copies to your local disk.

### Tips & Notes
- Navigating away from the "Files" page keeps SFTP sessions, directories, and active queues running in the background.`,
      },
      {
        id: 'files-transfer',
        title: 'Transfer Queue & Rate Limiting',
        badge: 'Queue',
        keywords: ['Transfer Queue', 'Rate Limit', 'Resume', 'Pause', 'Concurrency'],
        content: `### Overview
A comprehensive multi-task transfer scheduler with clear status tracking, throttling, and task lifecycle controls.

### Entry & Shortcuts
- **Open Queue Panel**: Click the "Transfer Queue" pill in the footer of the file manager.

### Core Steps
1. Active transfers display aggregate throughput and progress indicators in the footer.
2. Open the queue to individually pause, resume, or cancel queued items.
3. Configure global concurrency limits and upload/download speed caps to prevent bandwidth starvation.

### Tips & Notes
- Re-initiating interrupted transfers automatically checks target offsets for seamless resumption.`,
      },
      {
        id: 'files-dirs',
        title: 'Advanced Directory Operations',
        badge: 'Ops',
        keywords: ['Calculate Size', 'Fast Delete', 'Open in Terminal', 'du -sb', 'rm -rf'],
        content: `### Overview
Productivity accelerators designed for large-scale server administration: async directory size calculations, dual deletion modes, and one-click terminal jumps.

### Entry & Shortcuts
- **Action**: Right-click any file or directory in SFTP file lists.

### Core Steps
1. **Calculate Directory Size**: Right-click large folders and choose "Calculate Size"; an async \`du -sb\` computes space and child counts without blocking the UI.
2. **Fast Directory Deletion**: Choose "Delete" to select between:
   - *Shell Fast Delete* (Recommended): Dispatches optimized \`rm -rf\` over SSH, deleting tens of thousands of files in seconds.
   - *SFTP Step-by-Step*: Recursively deletes items via pure SFTP protocol, ideal for restricted environments.
3. **Open in Terminal**: Instantly creates a new terminal tab and automatically executes \`cd\` to that folder.

### Tips & Notes
- Fast shell deletion displays an impact confirmation prompt to protect critical system directories.`,
      },
      {
        id: 'files-perms',
        title: 'Permissions & Ownership (chmod / chown)',
        badge: 'Chmod',
        keywords: ['Permissions', 'chmod', 'chown', 'Grid', 'SUID', 'Directory Protection'],
        content: `### Overview
A complete Linux permissions and ownership dialog with read/write/execute grids, special bit flags, octal sync, and smart recursive directory protection.

### Entry & Shortcuts
- **Action**: Right-click a file or folder and choose "Properties / Permissions".

### Core Steps
1. Toggle permissions across Owner, Group, and Others in an intuitive 3x3 grid.
2. Set special flags including SUID, SGID, and Sticky Bit.
3. Interactive octal input (e.g. typing \`755\` or \`644\`) stays synchronized with the checkboxes.
4. Modify owner and group bindings (chown).
5. **Smart Directory Protection**: When applying recursively, choose "Smart Protection" to append execute flags (\`+X\`) only to directories, preventing locked-out subfolders.

### Tips & Notes
- The dialog header displays inode numbers, hard link counts, modification timestamps, and absolute paths.`,
      },
      {
        id: 'files-sudo',
        title: 'Sudo Escalation for Read & Write',
        badge: 'Sudo',
        keywords: ['Sudo', 'Escalation', 'Permission denied', 'root editing'],
        content: `### Overview
Solves the pain point of regular SSH accounts failing to edit protected configuration files under \`/etc/\` via SFTP.

### Entry & Shortcuts
- **Trigger**: Automatically prompts when encountering \`Permission denied\` while reading or writing files.

### Core Steps
1. When access is denied, a "Sudo Elevation" prompt requests your sudo password.
2. Enter your password (with an option to remember for the session).
3. The client leverages sudo pipes in the background to read the file and writes back via atomic temp-file swaps.

### Tips & Notes
- The entire process complies with standard Linux audit logs for traceability and compliance.`,
      },
      {
        id: 'files-diff',
        title: 'Dual-File Monaco Diff Editor',
        badge: 'Diff',
        keywords: ['Diff', 'Comparison', 'Monaco Diff', 'Side by Side', 'File Compare'],
        content: `### Overview
Embedded Monaco Diff Editor providing text comparison, real-time addition/deletion counters, and side-by-side or inline view switching.

### Entry & Shortcuts
- **Method 1**: Hold \`⌘\` (macOS) / \`Ctrl\` (Windows), select two files in the file list, and click "Compare Files".
- **Method 2**: Right-click the first file, choose "Select for Compare", then right-click the second file and choose "Compare with...".

### Core Steps
1. An inspection dialog opens with the base file on the left and target on the right.
2. **View Modes**: Switch between "Side-by-Side" and "Inline" mode from the toolbar.
3. **Swap Sides**: Click "Swap Sides" to reverse comparison direction.
4. Real-time statistics display added, deleted, and identical lines.

### Tips & Notes
- Supports comparing files across different remote hosts directly.`,
      },
    ],
  },
  {
    id: 'preview',
    title: 'Universal Preview Matrix',
    icon: 'Eye',
    sections: [
      {
        id: 'preview-markdown',
        title: 'Markdown Rich Preview',
        badge: 'Markdown',
        keywords: ['Markdown', 'MD', 'GFM', 'Tables', 'Task List', 'Render'],
        content: `### Overview
GFM-compliant rich text previewer supporting task lists, tables, syntax-highlighted code blocks with copy buttons, and three-state switching.

### Entry & Shortcuts
- **Trigger**: Double-click or right-click any \`.md\` file in SFTP file lists.

### Core Steps
1. Files render by default with styled headings, tables, strikethrough, and blockquotes.
2. Code blocks include language badges and a one-click copy button.
3. **Three-State View**: Toggle freely between "Rendered View", "Monaco Source", and "Hex View".

### Tips & Notes
- Designed for fast read-only viewing; switch to Source mode or open in the full editor when modifications are needed.`,
      },
      {
        id: 'preview-csv',
        title: 'CSV / TSV Virtualized Grid',
        badge: 'CSV',
        keywords: ['CSV', 'TSV', 'Grid', 'Virtual Scroll', 'Sort', 'Search'],
        content: `### Overview
Built-in RFC 4180 compliant delimiter engine featuring frozen headers, type-aware column sorting, real-time filtering, and viewport virtualization.

### Entry & Shortcuts
- **Trigger**: Double-click or right-click any \`.csv\` or \`.tsv\` file.

### Core Steps
1. Opens as an interactive data grid with frozen top headers and row counters.
2. **Sort Columns**: Click header cells to sort ascending/descending with automatic numeric vs text detection.
3. **Search**: Type in the top filter bar to instantly highlight and filter matching rows.
4. Viewport virtualization renders tens of thousands of rows smoothly with minimal memory.

### Tips & Notes
- Selected cells copy as TSV format for direct pasting into spreadsheet applications.`,
      },
      {
        id: 'preview-json',
        title: 'JSON Tree View & Built-in jq Filter',
        badge: 'JSON',
        keywords: ['JSON', 'JSONL', 'jq', 'Tree View', 'Collapse', 'JSONPath'],
        content: `### Overview
Supports standard JSON and JSON Lines (\`.jsonl\`). Features an interactive collapsible tree and an integrated native jq filtering engine.

### Entry & Shortcuts
- **Trigger**: Double-click or right-click any \`.json\` or \`.jsonl\` file.

### Core Steps
1. Parses into a colored tree distinguishing Keys, Strings, Numbers, Booleans, and Nulls.
2. Click fold arrows or use "Expand All" / "Collapse All" in the toolbar.
3. Hover over properties to copy their JSONPath, Key name, or Value.
4. **Built-in jq**: Enter jq expressions (e.g. \`.data[] | {id, name}\`) in the query bar and press Enter to filter dynamically.

### Tips & Notes
- Backend jq executions are guarded with a 3-second timeout and 5MB memory threshold against runaway recursions.`,
      },
      {
        id: 'preview-log',
        title: 'Large Log Smart Tail Mode',
        badge: 'Log',
        keywords: ['Log', 'Tail', 'Tail -f', 'Large Files', '256KB', 'Fast Open'],
        content: `### Overview
A dedicated rapid triage mode for multi-gigabyte server logs, loading the latest 256KB backward from the end of the file instantly.

### Entry & Shortcuts
- **Trigger**: Double-click or preview any \`.log\`, \`.out\`, or \`.err\` file.

### Core Steps
1. Instant inspection by reading the last 256KB of the file (aligning line cuts automatically).
2. **Follow Mode (Tail -f)**: Toggle live following to auto-scroll with a green pulsing heartbeat indicator.
3. **Level Filtering**: Filter quickly by ALL / ERROR / WARN / INFO.
4. **Historical Paging**: Click "Load Earlier Logs" to fetch the preceding block.

### Tips & Notes
- Bypasses traditional 5MB file transfer caps, allowing instant troubleshooting of massive production logs.`,
      },
      {
        id: 'preview-hex',
        title: 'Hex Viewer',
        badge: 'Hex',
        keywords: ['Hex', 'Binary', 'Disassembly', 'xxd', 'Byte Inspector'],
        content: `### Overview
A low-level diagnostic tool for binaries and unformatted files. Features a classic 16-byte three-column layout with chunked seek support.

### Entry & Shortcuts
- **Trigger**: Double-click binary files (\`.so\`, \`.bin\`, \`.exe\`, \`.db\`, etc.) or switch to Hex view in text previews.

### Core Steps
1. Left column shows byte offsets, center shows hexadecimal bytes, and right displays printable ASCII characters.
2. **Synchronized Selection**: Selecting a byte in the Hex view automatically highlights the corresponding character in the ASCII column.
3. **Byte Inspector**: The bottom bar decomposes the focused byte into Offset, Hex, Dec, Bin, and Char representations.
4. Export clean \`xxd\` style hexadecimal dumps with one click.

### Tips & Notes
- Streams data in 64KB seek chunks on demand, scrolling seamlessly through arbitrarily large files without high memory usage.`,
      },
      {
        id: 'preview-excel',
        title: 'Excel Multi-Sheet Workbook Preview',
        badge: 'Excel',
        keywords: ['Excel', 'xlsx', 'xls', 'Sheet', 'calamine', 'Spreadsheet'],
        content: `### Overview
A pure offline Excel workbook parser powered by the Rust \`calamine\` engine, previewing spreadsheets without requiring Microsoft Office.

### Entry & Shortcuts
- **Trigger**: Double-click or preview \`.xlsx\`, \`.xls\`, \`.xlsb\`, or \`.ods\` files.

### Core Steps
1. Opens workbook files with frozen top header rows.
2. Bottom tabs allow fast switching between multiple sheets with in-memory caching.
3. Supports column sorting, keyword search filtering, and cell range copying.

### Tips & Notes
- SFTP previews use managed temporary files that are safely purged when the preview window closes.`,
      },
      {
        id: 'preview-sqlite',
        title: 'SQLite Read-Only Browser & SQL Runner',
        badge: 'SQLite',
        keywords: ['SQLite', 'db', 'sqlite3', 'SQL', 'SELECT', 'Read-Only'],
        content: `### Overview
Safely inspects \`.db\`, \`.sqlite\`, and \`.sqlite3\` files in enforced read-only mode (\`SQLITE_OPEN_READ_ONLY\`), combining schema introspection with an SQL query console.

### Entry & Shortcuts
- **Trigger**: Double-click or preview any SQLite database file.

### Core Steps
1. Left panel enumerates all tables, views, and row counts.
2. Click any table to stream its columns and data into the virtualized grid.
3. **Execute SQL**: Run custom read-only statements (\`SELECT\`, \`PRAGMA table_info\`, \`EXPLAIN\`) in the top query bar.

### Tips & Notes
- The backend parser rejects modifying or destructive queries (\`INSERT\`, \`UPDATE\`, \`DROP\`) at the AST level to protect live databases.`,
      },
      {
        id: 'preview-cert',
        title: 'X.509 SSL Certificate & Key Inspector',
        badge: 'Cert',
        keywords: ['Certificate', 'SSL', 'TLS', 'x509', 'Expiry', 'SANs', 'Fingerprint'],
        content: `### Overview
Parses X.509 certificates using pure Rust cryptography libraries. Computes remaining validity days, displays chain hierarchies, SANs domains, and public key fingerprints.

### Entry & Shortcuts
- **Trigger**: Double-click or preview \`.crt\`, \`.cer\`, \`.pem\`, or \`.ca-bundle\` files.

### Core Steps
1. Validity status badge shows green (valid), yellow (expiring within 30 days), or red (expired) with remaining day counts.
2. **Chain Navigation**: Inspect root and intermediate certificate tiers within bundle files.
3. View Subject, Issuer, Serial Number, Not Before/After dates, Signature Algorithms, and Key Specs.
4. **Copy SANs**: View all Subject Alternative Names and copy domain lists with a single click.
5. Displays SHA-256 and SHA-1 fingerprints.

### Tips & Notes
- Handles multi-cert bundle files effortlessly for complete SSL trust path verification.`,
      },
      {
        id: 'preview-media',
        title: 'Images (including JXL) & Vector PDF Preview',
        badge: 'Media',
        keywords: ['Image', 'PNG', 'JPG', 'JXL', 'PDF', 'PDFium', 'JPEG XL'],
        content: `### Overview
A cross-platform viewer for visual and document assets. In addition to standard PNG, JPG, SVG, and WebP, it natively supports next-generation \`.jxl\` (JPEG XL) and vector PDF rendering.

### Entry & Shortcuts
- **Trigger**: Double-click any supported image or \`.pdf\` document.

### Core Steps
1. **Images**: Zoom, pan, inspect dimensions (width × height), and view color profiles.
2. **JPEG XL**: Uses built-in \`libjxl\` runtime bindings to decode ultra-compressed \`.jxl\` images.
3. **PDF**: Powered by Chromium's Google PDFium engine for crisp vector typography, paging, and scaling.

### Tips & Notes
- Native decoding libraries load dynamically, ensuring that missing optional libraries will never block core application boot.`,
      },
    ],
  },
  {
    id: 'editor',
    title: 'Code & Config Editor (Monaco Editor)',
    icon: 'FileCode2',
    sections: [
      {
        id: 'editor-core',
        title: '81 Offline Languages & Editor Core',
        badge: 'Core',
        keywords: ['Monaco', 'Editor', 'Offline Highlighting', 'Multi-Language', 'Save', 'Draft'],
        content: `### Overview
Bundles Microsoft's Monaco Editor core, providing offline syntax coloring for 81 programming and markup languages without external downloads.

### Entry & Shortcuts
- **Open File**: Right-click in SFTP and select "Open in Editor", or switch from the "Editor" sidebar icon.
- **Save**: Press \`⌘S\` (macOS) / \`Ctrl+S\` (Windows).

### Core Steps
1. Manage multiple open files in top tab cards.
2. Features code folding, bracket colorization, indent guides, multi-cursor editing (Alt+Click), and smart indentation.
3. Press \`⌘S\` / \`Ctrl+S\` to write modifications back to the remote server with toast confirmations.

### Tips & Notes
- Monarch tokenizer definitions are fully packaged for seamless offline productivity in restricted private networks.`,
      },
      {
        id: 'editor-nginx',
        title: 'Nginx Syntax & Context-Aware Completion',
        badge: 'Nginx',
        keywords: ['Nginx', 'conf', 'Context-Aware', 'http', 'server', 'location', 'Variables'],
        content: `### Overview
A custom Nginx language service offering multi-tier file detection, block-aware directive completion, variable completions, and official hover documentation.

### Entry & Shortcuts
- **Trigger**: Open any Nginx config file, files named \`.conf\`, or files under \`/nginx/\` directories.

### Core Steps
1. **Smart Detection**: Detects \`nginx.conf\`, paths containing \`/nginx/\`, or content containing blocks like \`events\`, \`http\`, \`server\`.
2. **Block-Aware Completion**: Recommends directives valid only within the current block (e.g. \`proxy_pass\` inside \`location\`).
3. **Variable Completion**: Type \`$\` to trigger auto-suggestions for built-in Nginx variables (e.g. \`$host\`, \`$remote_addr\`).
4. **Hover Docs**: Hover over directives to read official syntax explanations, contexts, and default values.

### Tips & Notes
- Files like \`fastcgi_params\` or \`mime.types\` without top-level blocks are correctly recognized via content heuristics.`,
      },
      {
        id: 'editor-docker',
        title: 'Dockerfile & Docker Compose Intelligence',
        badge: 'Docker',
        keywords: ['Docker', 'Dockerfile', 'docker-compose', 'Compose', 'BuildKit', 'Schema'],
        content: `### Overview
Deep language intelligence for container configs. Supports all 18 official Dockerfile instructions, BuildKit flags, and Compose service dependency completions.

### Entry & Shortcuts
- **Trigger**: Open files named \`Dockerfile\`, \`docker-compose.yml\`, or \`compose.yaml\`.

### Core Steps
1. **Dockerfile Directives**: Autocompletes official instructions, BuildKit flags, and predefined ARG variables.
2. **Multi-Stage Suggestions**: Typing \`COPY --from=\` parses and suggests prior \`AS <stage>\` aliases.
3. **Environment Expansion**: Typing \`$\` or \`\${\` prompts declared \`ENV\` and \`ARG\` identifiers.
4. **Compose Schema Validation**: Provides indent-aware key completions and suggests service names under \`depends_on\` and \`networks\`.

### Tips & Notes
- Compose completions run only on confirmed Docker Compose YAML files without interfering with generic CI or Kubernetes manifests.`,
      },
      {
        id: 'editor-bash',
        title: 'Bash/Shell Completion & Remote Environment Integration',
        badge: 'Shell',
        keywords: ['Bash', 'Shell', 'compgen', 'Functions', 'Variables', 'tree-sitter'],
        content: `### Overview
A hybrid client-server Shell editor combining Rust AST parsing with remote SSH \`compgen\` queries for real-world autocomplete.

### Entry & Shortcuts
- **Trigger**: Open any \`.sh\`, \`.bash\`, or \`.zsh\` script.

### Core Steps
1. **Keywords & Builtins**: Autocompletes standard control structures, reserved words, and common built-in commands.
2. **Document AST Symbols**: The backend parses scripts via \`tree-sitter-bash\` to suggest document-defined functions and variables.
3. **Remote Command Completion**: For remote SFTP files, the client queries available system binaries via \`compgen -c\` over SSH and merges them into suggestion lists.

### Tips & Notes
- Remote tool suggestions feature TTL caching and silent fallbacks so offline editing remains instantly responsive.`,
      },
      {
        id: 'editor-batch',
        title: 'Windows Batch Highlighting & Label Jumps',
        badge: 'Batch',
        keywords: ['Batch', 'bat', 'cmd', 'goto', 'call', 'Label Jump'],
        content: `### Overview
Dedicated language tooling for Windows batch scripts covering 316 commands, environment variable tokenization, and Goto label jumps.

### Entry & Shortcuts
- **Trigger**: Open \`.bat\` or \`.cmd\` files.

### Core Steps
1. Full syntax highlighting for \`::\` / \`REM\` comments, \`:label\` targets, \`%var%\`, \`!var!\`, and \`%%i\` loop variables.
2. Suggests Windows commands and standard execution snippets.
3. **Label Jump**: Right-click \`goto label\` or \`call :label\` references and choose "Go to Definition" (or press \`F12\`) to jump directly to the target.

### Tips & Notes
- Supports label renaming, propagating changes across all call references across the document.`,
      },
      {
        id: 'editor-sql',
        title: 'SQL Keywords, Dialects & Functions',
        badge: 'SQL',
        keywords: ['SQL', 'MySQL', 'PostgreSQL', 'SQLite', 'Functions', 'Keywords'],
        content: `### Overview
Integrates 349 ANSI SQL keywords and popular database dialect extensions with nearly 500 built-in functions and context pruning.

### Entry & Shortcuts
- **Trigger**: Open any \`.sql\` script.

### Core Steps
1. Type to trigger uppercase SQL keyword suggestions (e.g. \`SELECT\`, \`UPDATE\`, \`WHERE\`, \`GROUP BY\`).
2. Provides dialect keywords for MySQL 8, PostgreSQL 13, and SQLite 3.
3. **Context Pruning**: Detects clauses to suppress arithmetic function suggestions in table-reference contexts (e.g. following \`FROM\`, \`JOIN\`, \`INTO\`).

### Tips & Notes
- Includes DDL and DML scaffolding snippets; type \`create_table\` to expand clean boilerplate tables.`,
      },
      {
        id: 'editor-systemd',
        title: 'Systemd Unit Highlighting & Directive Enum Suggestions',
        badge: 'Systemd',
        keywords: ['Systemd', 'service', 'unit', 'Restart', 'ExecStart', 'Enums'],
        content: `### Overview
Language support for Linux system service configs (\`.service\`, \`.unit\`, \`.target\`). Features section-aware directive autocompletion and enum value suggestions.

### Entry & Shortcuts
- **Trigger**: Open files ending with \`.service\`, \`.socket\`, \`.target\`, or \`.timer\`.

### Core Steps
1. Section headers (e.g. \`[Unit]\`, \`[Service]\`, \`[Install]\`) and specifiers like \`%i\` and \`%h\` receive distinct theme colors.
2. **Section-Awareness**: Inside \`[Service]\`, suggestions only include valid service directives (such as \`ExecStart\`, \`Restart\`, \`User\`).
3. **Enum Values**: Typing \`=\` after directives prompts valid options (e.g. \`Restart=\` prompts \`always\`, \`on-failure\`, \`no\`).

### Tips & Notes
- Hovering over directives reveals descriptions and section classifications from official systemd documentation.`,
      },
      {
        id: 'editor-crontab',
        title: 'Crontab Expression Validation & Hover Next-Run Previews',
        badge: 'Cron',
        keywords: ['Crontab', 'Cron', 'Schedule', 'Hover Preview', 'Next Run', 'Validation'],
        content: `### Overview
An intelligent parser for Linux scheduled tasks adhering to standard Vixie Cron specifications, complete with error squiggles and next execution previews.

### Entry & Shortcuts
- **Trigger**: Open crontab config files or files named \`crontab\`.

### Core Steps
1. Supports 5-field schedules, step intervals (\`/2\`), numeric ranges, month/day names, and \`@reboot\`/\`@daily\` macros.
2. **Error Squiggles**: Invalid fields (such as minute values exceeding 59) are marked with red squiggly underlines.
3. **Hover Next-Run Preview**: Hovering over valid cron expressions dynamically calculates and displays the **next 3 runtimes in your local timezone**.

### Tips & Notes
- Insert standard schedule templates instantly without needing external online cron testers.`,
      },
      {
        id: 'editor-html',
        title: 'HTML / CSS Smart Tag Auto-Closing & Quote Pairing',
        badge: 'HTML',
        keywords: ['HTML', 'CSS', 'Auto Close', 'Tag Pairing', 'Emmet', 'Attributes'],
        content: `### Overview
Integrates VS Code language service workers for complete HTML5, CSS3, SCSS, and LESS tag intelligence and typing assistance.

### Entry & Shortcuts
- **Trigger**: Open \`.html\`, \`.htm\`, or \`.css\` files.

### Core Steps
1. **Auto-Close Tag**: Typing \`>\` after \`<div>\` automatically inserts the closing \`</div>\` and centers your cursor.
2. **Close Unfinished Tag**: Typing \`</\` looks back to find and complete the nearest open tag name.
3. **Attribute Quote Pairing**: Typing \`=\` after attributes automatically inserts matching quotes \`""\` and positions the cursor inside.

### Tips & Notes
- Language workers load asynchronously as standalone chunks, consuming zero memory when HTML/CSS files are not open.`,
      },
      {
        id: 'editor-draft',
        title: 'Debounced Drafts & Remote Conflict Resolution',
        badge: 'Safety',
        keywords: ['Drafts', 'Conflict Resolution', 'Concurrency', 'SQLCipher', 'Hash Diff'],
        content: `### Overview
Safety mechanisms that debounce edits into local encrypted storage while using remote content hashes to negotiate concurrent multi-user conflicts.

### Entry & Shortcuts
- **Trigger**: Runs continuously in the background during editing.

### Core Steps
1. Keystrokes are debounced and committed to the encrypted \`editor_drafts\` SQLite table.
2. In the event of a power outage or accidental closure, re-opening the file restores your unsaved draft.
3. **Conflict Arbitration**: Before saving, the client compares current remote hashes. If modified upstream, a conflict dialog prompts you to:
   - *Overwrite Remote*: Force-write local edits over the remote copy.
   - *Discard Draft*: Discard local edits and pull the latest remote version.

### Tips & Notes
- You can review side-by-side differences in the Monaco Diff Editor before confirming an overwrite.`,
      },
    ],
  },
  {
    id: 'network',
    title: 'Remote Network & Topology (Relay, ZK, gRPC, Port Forward)',
    icon: 'Network',
    sections: [
      {
        id: 'network-relay',
        title: 'VLR2 Self-Hosted SSH Relay Management & Latency Ping',
        badge: 'Relay',
        keywords: ['Relay', 'VLR2', 'Tunnel', 'TCP', 'WebSocket', 'QUIC', 'Bastion'],
        content: `### Overview
Self-hosted high-performance SSH relay system powered by the proprietary VLR2 protocol. Uses dual-nonce challenge-response and HMAC signing over TCP, WebSocket, or QUIC to traverse restricted enterprise networks.

### Entry & Shortcuts
- **Manage Relays**: Click "Relays" on the sidebar.
- **Bind to Host**: In host settings, check "Connect via Relay" and pick an active node.

### Core Steps
1. In the Relays screen, click "New Relay" to set server address, port, transport mode (TCP / WS / QUIC), and credentials (Key ID + Secret).
2. Click "Test Connection" on any node to initiate an instant handshake and measure round-trip latency in milliseconds.
3. Link the relay to any host; all terminal sessions, SFTP operations, and port forwards to that host will tunnel through the relay.

### Tips & Notes
- The 32-byte shared secret is never transmitted over the wire, protecting against eavesdropping and replay exploits.`,
      },
      {
        id: 'network-topology',
        title: 'Single-Host & Global Network Topology Visualizer',
        badge: 'Topology',
        keywords: ['Topology', 'Chains', 'Multi-Hop', 'Visualizer', 'Network Overview'],
        content: `### Overview
Visualizes complex proxy topographies and connection paths: Local -> Relay Node -> Multi-hop Jump Hosts -> Target Host -> Port Forwards.

### Entry & Shortcuts
- **Single-Host Topology**: Click the "Topology" network icon on any host card.
- **Global Overview**: Click "Global Topology" in the top toolbar of the Hosts screen.

### Core Steps
1. **Single-Host View**: Displays all proxy tiers and attached port forwards with live status indicators (Idle, Connecting, Active, Error).
2. **Global Topology Canvas**: Aggregates all configured hosts in a unified graph, pinning active sessions to the top.
3. Click any node card in the canvas to inspect its status or initiate a session directly.

### Tips & Notes
- Toggle attached port forwarding rules directly inside the topology dialog without switching screens.`,
      },
      {
        id: 'network-forward',
        title: 'Local & Remote SSH Port Forwarding',
        badge: 'Port',
        keywords: ['Port Forwarding', 'SSH Tunnel', 'Local Forward', 'Tunnel Mapping'],
        content: `### Overview
Manages secure SSH port forwarding tunnels, allowing you to expose local dev ports into remote bastions or map private remote endpoints back to localhost.

### Entry & Shortcuts
- **Action**: Click the "Port Forward" icon on any host card or configure within host topology views.

### Core Steps
1. Click "Add Rule" and define the local listening port (e.g. \`8080\`) and destination target (e.g. \`10.0.0.5:80\`).
2. Configure rules to start automatically with sessions or toggle them manually on demand.
3. Review uptime metrics and port conflict warnings in the rule list.

### Tips & Notes
- Port forwarding stacks seamlessly with multi-hop jump hosts and relays, reaching deep into isolated multi-tier networks.`,
      },
      {
        id: 'network-zk',
        title: 'ZooKeeper Distributed Node Browser',
        badge: 'ZK',
        keywords: ['ZooKeeper', 'ZK', 'znode', 'Dubbo', 'ACL', 'Watcher', 'Stat'],
        content: `### Overview
A centralized management console for ZooKeeper clusters featuring lazy-loaded znode trees, data editing, Stat metrics inspection, ACLs, and live watchers.

### Entry & Shortcuts
- **Enter ZK**: Click "ZK Hosts" to manage cluster endpoints, then click "ZooKeeper" to explore trees.

### Core Steps
1. **Tree Exploration**: Lazy-loads znodes with hierarchy expansion and keyword search.
2. **Data Inspection**: View and edit payload data in UTF-8 text or raw Hex modes.
3. **Stat Metadata**: Inspect 11 core attributes including \`czxid\`, \`mzxid\`, \`ctime\`, \`mtime\`, \`version\`, and \`ephemeralOwner\`.
4. **ACL Controls**: Configure CDRWA permissions (Create, Delete, Read, Write, Admin) visually.
5. **Create & Delete**: Create persistent, ephemeral, and sequential znodes; delete nodes individually or cascade recursively.
6. **Dubbo Integration**: Recognizes Dubbo registry URLs and provides an "Open in gRPC" shortcut.

### Tips & Notes
- Save digest authentication credentials in the credentials manager to automate authentication on protected clusters.`,
      },
      {
        id: 'network-grpc',
        title: 'gRPC Server Reflection Debugger',
        badge: 'gRPC',
        keywords: ['gRPC', 'Server Reflection', 'Proto', 'mTLS', 'API Testing', 'Reflection'],
        content: `### Overview
A lightweight gRPC interactive test bench supporting official Server Reflection, automated Protobuf payload scaffolding, and mTLS certificate management.

### Entry & Shortcuts
- **Open Debugger**: Click "gRPC" on the sidebar or click "Open in gRPC" from ZooKeeper Dubbo nodes.

### Core Steps
1. Enter the target gRPC endpoint (e.g. \`127.0.0.1:50051\`).
2. Click "Fetch Services"; the client introspects all available services and methods via Server Reflection.
3. Pick a method; the client generates a boilerplate JSON request payload with schema field comments.
4. **Security**: Toggle TLS, supply custom Server Names, and attach CA certs, client certs, and private keys for mTLS.
5. Click "Invoke" to review execution latency, status codes, and formatted response JSON.

### Tips & Notes
- Form inputs and certificate assignments persist across page navigation.`,
      },
    ],
  },
  {
    id: 'dbworkbench',
    title: 'Database Workbench (Database)',
    icon: 'Database',
    sections: [
      {
        id: 'db-connections',
        title: 'Connection Library & Testing',
        badge: 'MySQL/PG/DM',
        keywords: ['Database', 'MySQL', 'PostgreSQL', 'Oracle', 'Dameng', 'DM8', 'Compatibility', 'Connection', 'SSL', 'TLS', 'Read-only'],
        content: `### Overview
Built-in connectivity for MySQL, PostgreSQL, Oracle and Dameng DM8 (MySQL/PG use pure-Rust async drivers, Oracle loads ODPI-C at runtime, Dameng uses the vendored pure-Rust DM8 protocol driver). Passwords live in the SQLCipher-encrypted store, with environment tiers and a read-only guard toggle.

### Entry & Shortcuts
- **Entry**: Create or edit connections on the "DB Hosts" page in the sidebar.

### Core Steps
1. Click "New Connection" and pick an engine (MySQL 3306 / PostgreSQL 5432 / Oracle 1521 / Dameng 5236; the default port is auto-filled).
2. Fill in host, port, database, username and password; choose an SSL mode:
   - **Disable**: plaintext;
   - **Prefer/Require TLS**: encrypted without certificate verification (self-signed intranet certs);
   - **Verify Full**: validation against system root certificates (managed cloud databases).
   - **Dameng does not support TLS** (the driver only encrypts login); this selector is disabled for it.
3. Set the environment tier (dev/test/staging/prod) and the "Read-only session" toggle (on by default: writes are rejected by a backend AST whitelist).
4. "Test" before saving: connect, fetch the server version, disconnect, and report the round-trip in milliseconds.

### Dameng DM8 Notes
- **Compatibility mode**: whether a Dameng instance is MySQL- or Oracle-compatible is decided server-side by \`COMPATIBLE_MODE\` in \`dm.ini\`; clients cannot switch it. The form's "Compatibility" option only affects the **client-side SQL dialect** (statement splitting, read-only checks, ROWNUM vs LIMIT, FROM DUAL in the starter query). A wrong choice never breaks the connection.
- **Initial schema**: the "database" field means the default schema after login (Oracle's CURRENT_SCHEMA); it may be left empty.
- **Catalog tree**: in both compatibility modes the tree uses Dameng's native Oracle-style dictionaries (\`ALL_USERS\` / \`ALL_TABLES\` / \`ALL_VIEWS\` / \`ALL_OBJECTS\` / \`ALL_SEQUENCES\`) — compatibility mode only adds to these views, never removes them.
- **Zero external dependencies**: the driver is a pure-Rust protocol implementation; no Dameng Instant Client or libdmdpi needed.

### Tips & Notes
- On edit, an empty password field keeps the stored one (passwords are never echoed back).
- The read-only guard runs on the backend: writes are rejected even if the frontend is bypassed.`,
      },
      {
        id: 'db-workbench',
        title: 'SQL Workbench & Catalog Tree',
        badge: 'SQL',
        keywords: ['SQL Workbench', 'Query', 'Catalog', 'Schema', 'Cancel', 'Result Sets'],
        content: `### Overview
A multi-session tab workbench: a lazily loaded catalog tree on the left (database/schema → tables/views/functions/procedures/sequences groups with counts, empty groups hidden → columns of tables/views; resizable), a Monaco SQL editor (static completion) and streaming result grids (per-statement tabs, sorting, filtering, TSV copy) on the right.

### Entry & Shortcuts
- **Entry**: Press ▶ on a connection in "DB Hosts", or click + in the workbench and pick a saved connection.
- **Run**: ⌘/Ctrl + Enter; cancel and row limit (1,000–50,000) live on the toolbar.

### Core Steps
1. Connecting loads the catalog tree; expand a database/schema → table/view/function/procedure/sequence groups, then a table to inspect columns and types.
2. **Double-click a table** to draft a \`SELECT * ... LIMIT 200\` into the editor.
3. Write SQL (multiple statements split on semicolons; semicolons inside strings/comments are ignored) and run with ⌘/Ctrl+Enter.
4. The result panel tabs per statement: result grid (sortable/filterable/copyable), DML affected-row counts, and error details; large results stream in 300-row batches.
5. Cancel long-running queries at any time (the backend streaming loop aborts immediately).

### Tips & Notes
- Page/tab switches preserve all session state (the workbench stays mounted); closing a tab disconnects.
- In read-only sessions, writes are intercepted before execution with the reason shown.`,
      },
    ],
  },
  {
    id: 'hosts',
    title: 'Hosts & Asset Management (Hosts & Groups)',
    icon: 'Server',
    sections: [
      {
        id: 'hosts-manage',
        title: 'Multi-Tier Host Groups & Environments',
        badge: 'Asset',
        keywords: ['Hosts', 'Assets', 'Groups', 'Environments', 'PROD', 'Labels', 'Config'],
        content: `### Overview
A structured server asset management hierarchy supporting unlimited nested groups, fuzzy searching, and environment classifications.

### Entry & Shortcuts
- **Manage Assets**: Click "Hosts" or "Groups" on the sidebar.
- **Fast Search**: Use the search bar in the Hosts list to filter by name, IP, tag, or username.

### Core Steps
1. Build hierarchical folder trees in "Groups" (e.g. by region, project, or department).
2. Add servers in "Hosts": configure label, IP/hostname, SSH port, username, and credentials.
3. **Environment Tiers**: Tag machines as Dev, Test, Staging, or Prod; tagging as Prod enables safety guard stripes and confirmation barriers.
4. Right-click host cards to copy connection strings or generate quick launch shortcuts.

### Tips & Notes
- Assign custom color themes per host so multiple concurrent sessions stand out visually.`,
      },
      {
        id: 'hosts-password-history',
        title: 'Host Password History Vault',
        badge: 'Backup',
        keywords: ['Password History', 'Recover Password', 'Backup', 'Vault'],
        content: `### Overview
An automated credential history vault. Every time host passwords are changed manually or through SSH password updates, previous credentials are encrypted and preserved (retaining up to 20 revisions).

### Entry & Shortcuts
- **View History**: In the host edit form, click the clock icon beside the password field.

### Core Steps
1. Click the clock icon next to the password input to expand the history drawer.
2. The drawer lists past password timestamps and masked values.
3. Copy historic passwords to your clipboard or click "Restore This Password" to roll back instantly.

### Tips & Notes
- History records remain strictly inside the local encrypted SQLCipher database and are never transmitted to external services.`,
      },
      {
        id: 'hosts-ready-hook',
        title: 'SSH Terminal Ready Hook',
        badge: 'Hook',
        keywords: ['Ready Hook', 'Auto Command', 'Init', 'Delay Injection'],
        content: `### Overview
Automatically dispatches designated commands to the remote session once the SSH handshake completes and initial screen output settles.

### Entry & Shortcuts
- **Setup**: In host configuration, expand "Advanced Options" -> "Ready Hook".

### Core Steps
1. Select a saved command from your "Commands Library" (e.g. loading environment modules, navigating to project folders, checking status).
2. Save the host and connect.
3. The client waits for the connection to establish and injects the command after 500ms of initial output silence.

### Tips & Notes
- The 500ms quiet window prevents race conditions that could swallow commands during MOTD printing or cause duplicate echoes.`,
      },
    ],
  },
  {
    id: 'commands',
    title: 'Commands Library & Global Palette',
    icon: 'TerminalSquare',
    sections: [
      {
        id: 'commands-library',
        title: 'Command Library & Parameterized Templates',
        badge: 'Library',
        keywords: ['Command Library', 'Templates', 'Placeholders', 'Prebuilt Commands', 'Snippets'],
        content: `### Overview
A centralized command knowledge base containing 192 pre-verified commands from official GNU and Microsoft manuals, with dynamic parameter templates via \`{{param}}\`.

### Entry & Shortcuts
- **Open Library**: Click "Commands" on the sidebar.

### Core Steps
1. Explore 192 built-in commands covering Linux files, text, system, networking, and Windows cmd operations.
2. **Create Templates**: Use double curly braces to declare input placeholders (e.g. \`docker logs -n {{lines}} {{container}}\`).
3. **Execute**: Click "Run" on any card to open a parameter popup, then send to the active terminal or execute in the background.
4. Export your command library as JSON or batch import curated command sets.

### Tips & Notes
- Built-in commands feature verified citations and safety flags, warning you before running destructive commands.`,
      },
      {
        id: 'commands-palette',
        title: 'Global Command Palette',
        badge: 'Palette',
        keywords: ['Command Palette', 'Quick Search', 'Shortcut', 'Launcher'],
        content: `### Overview
A modern global launcher modal inspired by VS Code. Press a shortcut from anywhere in the app to search and execute commands instantly.

### Entry & Shortcuts
- **Open Shortcut**: Press \`⌘K\` (macOS) / \`Ctrl+K\` (Windows) globally.

### Core Steps
1. Press \`⌘K\` / \`Ctrl+K\` anywhere in the application to summon the centered search palette.
2. Type keywords to match against command titles and syntax.
3. Use Arrow keys to navigate results, press Enter to dispatch to the active terminal, and press Esc to dismiss.

### Tips & Notes
- Supports in-place parameter filling without closing the palette window.`,
      },
    ],
  },
  {
    id: 'tools',
    title: 'Windows Embedded Toolchain (Windows BusyBox)',
    icon: 'Wrench',
    sections: [
      {
        id: 'tools-coreutils',
        title: '131 Built-in Commands via Single Binary',
        badge: 'Coreutils',
        keywords: ['BusyBox', 'Coreutils', 'Windows', 'ls', 'grep', 'sed', 'awk', 'Single Binary'],
        content: `### Overview
> **Note: This module is active only on Windows.**
Brings a comprehensive Unix toolkit to Windows by compiling mainstream POSIX utilities into a single, self-contained \`coreutils.exe\`.

### Entry & Shortcuts
- **Usage**: Type command names directly into VeloTerm's local Windows terminal.

### Core Steps
1. Open a local terminal and run standard Unix commands:
   - Basics: \`ls\`, \`cat\`, \`grep\`, \`sed\`, \`find\`, \`tar\`, \`curl\`, \`wget\`, \`tree\`...
   - Text Processing: \`head\`, \`tail\`, \`cut\`, \`sort\`, \`uniq\`, \`wc\`, \`tr\`, \`split\`...
   - System Monitoring: \`ps\`, \`top\`, \`df\`, \`du\`, \`free\`, \`uptime\`, \`which\`...
2. Ready out of the box without requiring Git Bash, Cygwin, or MSYS2.

### Tips & Notes
- The BusyBox architecture maintains a tiny footprint, instant startup, and strict GNU command-line compatibility.`,
      },
      {
        id: 'tools-enhanced',
        title: 'Enhanced Utilities (rg / fzf / jq / yq / nmap / vim)',
        badge: 'Heavy',
        keywords: ['ripgrep', 'fzf', 'jq', 'yq', 'nmap', 'vim', 'bc', 'qalc', 'openssl'],
        content: `### Overview
Includes 20+ heavyweight tools compiled directly as native DLLs and statically linked binaries alongside standard coreutils.

### Entry & Shortcuts
- **Usage**: Run directly from the local cmd prompt.

### Core Steps
1. **Search & Fuzzy Find**: Built-in \`rg\` (ripgrep) and interactive terminal fuzzy-finder \`fzf\`.
2. **Data Parsing**: C-native \`jq\` 1.8.2 and Go-built \`yq\` v4.54.1.
3. **Network Scanning**: Official MSVC builds of \`nmap\` 7.992 (with delay-load fallback to connect scanning if Npcap is absent), \`ncat\`, \`nping\`, and complete \`openssl\` CLI.
4. **Console Editing**: Built-in official VIMDLL MSVC build of \`vim\` 9.2 with stripped runtime files; run \`vim <file>\` to edit right inside the terminal.
5. **Calculators & Navigation**: High-precision \`bc\`, symbolic calculator \`qalc\`, and \`zoxide\` smart directory jumper.

### Tips & Notes
- All enhanced binaries are patched for clean UTF-8 console output, avoiding garbled characters.`,
      },
      {
        id: 'tools-hardlinks',
        title: 'NTFS Hardlinks & Automatic PATH Injection',
        badge: 'NTFS',
        keywords: ['Hardlinks', 'Short Names', 'PATH', 'tools', 'Automatic'],
        content: `### Overview
Automatically provisions NTFS hardlinks for all built-in commands inside the application's \`tools/\` directory and prepends it to the terminal PATH.

### Entry & Shortcuts
- **Operation**: Runs automatically during VeloTerm startup.

### Core Steps
1. On boot, the app checks file identity and links over 130 command short names (e.g. \`ls.exe\`, \`grep.exe\` pointing to \`coreutils.exe\`).
2. Prepends the \`tools/\` directory to the terminal child process \`PATH\`.
3. Use short command names seamlessly in cmd without referencing host binaries.

### Tips & Notes
- Link reconciliation is idempotent, updating only after application version upgrades.`,
      },
    ],
  },
  {
    id: 'settings',
    title: 'System Settings & Security (Settings & Safety)',
    icon: 'ShieldCheck',
    sections: [
      {
        id: 'settings-theme',
        title: 'Theme Appearances & Design Tokens',
        badge: 'Theme',
        keywords: ['Theme', 'Dark', 'Light', 'Font Size', 'UI Scaling'],
        content: `### Overview
A unified interface design token system supporting Dark, Light, and System modes, with coordinated terminal and editor palettes.

### Entry & Shortcuts
- **Open Settings**: Click the gear icon at the bottom-left of the sidebar, or press \`⌘,\` (macOS) / \`Ctrl+,\` (Windows).

### Core Steps
1. Open "Settings" -> "Appearance": choose Dark, Light, or System.
2. **UI Scaling**: Adjust the global font scale; all cards, sidebars, and forms scale proportionally.
3. **Terminal Palettes**: Customize ANSI 16-color schemes per host, accurately reflected in recording and playback.

### Tips & Notes
- Monaco Editor, xterm instances, and code previewers update theme styles simultaneously in the same frame without reloading.`,
      },
      {
        id: 'settings-sidebar',
        title: 'Sidebar Navigation Visibility & Reordering',
        badge: 'Sidebar',
        keywords: ['Sidebar', 'Menu Customization', 'Drag Reorder', 'Collapse'],
        content: `### Overview
Customizable navigation sidebar allowing you to hide rarely used screens and drag to reorder navigation icons.

### Entry & Shortcuts
- **Collapse/Expand Sidebar**: Click the top-left toggle or press \`⌘B\` (macOS) / \`Ctrl+B\` (Windows).
- **Customize**: Open "Settings" -> "Sidebar Visibility".

### Core Steps
1. In the sidebar visibility table, toggle checkboxes to show or hide modules (e.g. gRPC, ZooKeeper, Relays).
2. Drag row handles up and down to reorder icons; the sidebar updates and persists changes immediately.
3. **Resize Width**: Hover over the right edge of the sidebar and drag horizontally to adjust width.

### Tips & Notes
- The "Settings" icon remains permanently pinned at the bottom to guarantee access to preferences.`,
      },
      {
        id: 'settings-logging',
        title: 'Runtime Log Levels & Physical Log Inspection',
        badge: 'Log',
        keywords: ['Log Level', 'Troubleshooting', 'Debug', 'Trace', 'Log Path'],
        content: `### Overview
Change log verbosity dynamically at runtime and locate on-disk diagnostic log files with a single click.

### Entry & Shortcuts
- **Action**: Open "Settings" -> "Diagnostics & Logs".

### Core Steps
1. Switch between \`Trace\`, \`Debug\`, \`Info\`, \`Warn\`, and \`Error\` from the dropdown without restarting.
2. View the absolute filesystem path of the current log file (e.g. \`~/Library/Logs/veloterm/\` on macOS).
3. Click "Reveal in File Manager" to open the log directory directly.

### Tips & Notes
- Log entries follow standardized ISO timestamps, level markers, and module tags for easy support sharing.`,
      },
      {
        id: 'settings-known-hosts',
        title: 'Known Hosts Key Fingerprint Management',
        badge: 'Security',
        keywords: ['Known Hosts', 'Fingerprint', 'SSH Security', 'Man-in-the-Middle', 'Reset Keys'],
        content: `### Overview
Manage trusted host public key fingerprints and key types to protect against MITM attacks or handle host re-installations.

### Entry & Shortcuts
- **Action**: Open "Settings" -> "Security & Trust".

### Core Steps
1. Inspect saved host IP, port, key algorithm (e.g. \`ssh-ed25519\`), and full SHA-256 fingerprints.
2. If a server is re-installed, locate its entry and click "Revoke" to clear stale fingerprints cleanly.
3. Click "Clear All Fingerprints" during network troubleshooting if needed.

### Tips & Notes
- Connecting to unknown fingerprints always triggers an interactive confirmation prompt.`,
      },
      {
        id: 'settings-encryption',
        title: 'SQLCipher 256-Bit Encrypted Storage Architecture',
        badge: 'Cipher',
        keywords: ['SQLCipher', 'Encryption', 'AES-256', 'Credential Security', 'Window Restore'],
        content: `### Overview
Hardened data security. All credentials, private keys, session drafts, command histories, and window positions are stored in a local AES-256 encrypted SQLite database.

### Entry & Shortcuts
- **Operation**: Transparent automated background encryption.

### Core Steps
1. Unlocks \`veloterm.db\` in the application data directory on startup.
2. Sensitive fields (passwords, private keys, relay secrets, drafts) are encrypted before hitting disk, protecting against cold-disk extraction.
3. **Window Geometry Restoration**: Window position and size changes are debounced and committed to SQLite, restoring your exact layout upon reopening.

### Tips & Notes
- Table creations and migrations are strictly idempotent, guaranteeing safe and lossless upgrades across versions.`,
      },
    ],
  },
];
