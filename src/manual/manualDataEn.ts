import type { ManualChapter } from './manualTypes';

export const manualChaptersEn: ManualChapter[] = [
  {
    id: 'terminal',
    title: 'Terminal',
    icon: 'Terminal',
    sections: [
      {
        id: 'terminal-pty',
        title: 'Local terminal',
        badge: 'Local',
        keywords: ['cmd', 'zsh', 'bash', 'cd', 'local terminal', 'switch drive', 'll'],
        content: `### What it does
Opens a command line window on your own computer. On Mac and Linux it uses your usual command line (zsh, bash or fish); on Windows it opens the standard command prompt (cmd).

### Where to find it
- The "+" button at the top of the window, or \`⌘N\` (Mac) / \`Ctrl+N\` (Windows).
- The "New Local Terminal" card on the welcome screen.

### How to use it
1. Open a terminal — it starts in your home folder.
2. On Windows, switch to another drive simply by typing the destination, e.g. \`cd D:\\folder\` or \`chdir E:\\project\`.
3. Type \`ll\` as a shortcut for a detailed file listing (it runs \`ls -lh\`).

### Tips
- The Windows drive switch needs no extra steps or special typing — just name the folder you want.
- Colors, symbols and emoji look the same as on Mac and Linux.`,
      },
      {
        id: 'terminal-ssh',
        title: 'Connecting to remote servers (SSH)',
        badge: 'SSH',
        keywords: ['ssh', 'connect', 'password', 'private key', 'jump host', 'proxyjump', 'agent', 'rz', 'sz', 'reconnect'],
        content: `### What it does
Connects you to remote servers so you can work on them as if you were sitting at them. Sign in with a password or a private key, pass through gateway (jump) servers, send and receive files right inside the terminal, and reconnect automatically after a network hiccup.

### Where to find it
- Hosts list on the left: double-click a host card, or click its "Terminal" icon.
- "Reconnect" icon in the top toolbar, or \`⌘R\` (Mac) / \`Ctrl+R\` (Windows).

### How to use it
1. Sign in with a password, a private key file (its passphrase is remembered), or your SSH agent.
2. For hard-to-reach servers, set up a chain of jump hosts in the host settings.
3. Move files without leaving the terminal: type \`sz <filename>\` to send a file to your computer, or \`rz\` to receive one — a progress panel appears on its own.

### Tips
- If the server forces a password change at login, you are guided through it and the new password is saved.
- Reconnecting keeps your on-screen history and marks the break with a "-- Reconnected --" line. The first connection to a new server asks you to confirm you trust it.`,
      },
      {
        id: 'terminal-layout',
        title: 'Terminal tabs and grid view',
        badge: 'Layout',
        keywords: ['tabs', 'grid', 'split', 'switch tab', 'reorder', 'rename'],
        content: `### What it does
Keep several terminals open at once — either switched as tabs, or shown all together side by side in a grid.

### Where to find it
- New tab: the "+" in the tab bar, or \`⌘T\` / \`Ctrl+T\`. Close: the "×" on the tab, or \`⌘W\` / \`Ctrl+W\`.
- Jump to a tab: \`⌘1\`–\`⌘9\` (Mac) / \`Ctrl+1\`–\`Ctrl+9\` (Windows).
- Grid/tabs toggle: the grid icon on the right side of the terminal header.

### How to use it
1. Open several sessions — tabs show connection status, PROD/STAGE labels and a recording dot.
2. Drag tabs to reorder them; double-click a tab title to give it a temporary name.
3. Click the grid icon to show every open session at once, each with its own scrolling.

### Tips
- In grid view, typing goes only to the pane you click. To type on many servers at once, use Broadcast (next section).`,
      },
      {
        id: 'terminal-broadcast',
        title: 'Broadcast: typing on many terminals at once',
        badge: 'Sync',
        keywords: ['broadcast', 'multiple servers', 'same command', 'sync input', 'ctrl+c'],
        content: `### What it does
Sends what you type to several terminals at the same time, from one input box — handy when many servers need the same command.

### Where to find it
- The "Broadcast" icon in the terminal toolbar.

### How to use it
1. Click the icon to open the broadcast bar at the bottom.
2. Tick which open terminals should listen (you can select by server or by environment).
3. Type a command and press Enter — it goes to every selected terminal at once.
4. Use "Send Ctrl+C" to stop runaway commands everywhere, or "Clear Screen" to refresh all panes.

### Tips
- Double-check you are in the right folder with the right permissions before sending a risky command, especially on production servers.`,
      },
      {
        id: 'terminal-monitor',
        title: 'Server health at a glance',
        badge: 'Monitor',
        keywords: ['resource monitor', 'cpu', 'memory', 'disk', 'network', 'swap', 'latency'],
        content: `### What it does
Shows live health figures for the server you are connected to — response time, CPU, memory, network speed, disk usage — in a strip above the terminal.

### Where to find it
- The pulse-wave "Resource Monitor" icon in the terminal toolbar (off by default).

### How to use it
1. Click the icon to turn it on for the current SSH session.
2. Figures refresh every 2 seconds: response time, CPU %, memory %, swap %, network send/receive speed, root disk usage %, and the system name.
3. In a narrow window, less important figures tuck themselves away until there is room again.

### Tips
- Only the terminal you are actually looking at is checked, so idle sessions never burden the server.`,
      },
      {
        id: 'terminal-recording',
        title: 'Recording what happens in the terminal',
        badge: 'Record',
        keywords: ['record', 'playback', 'replay', 'gif', 'mp4', 'cast', 'video', 'export'],
        content: `### What it does
Records a terminal session so you can replay it later like a video, or save it as a video (.gif, .mp4), a plain text log (.log), or a .cast file for playback websites.

### Where to find it
- Start: right-click inside the terminal → "Start Recording Session".
- The pulsing red REC pill at the top-left opens the controls.
- Replay: right-click → "Replay Last Recording", or use the pill menu after stopping.

### How to use it
1. Start recording and work as usual — only what appears on screen is captured; passwords you type stay hidden.
2. Stop from the pill; a small panel then offers to play it back (speeds 0.5x–4x, quiet parts skipped automatically) or export it as .cast, .log, GIF or MP4.

### Tips
- GIF and MP4 creation first shows a progress screen, then a save window.
- .cast files can be uploaded to asciinema.org or played with its tools.`,
      },
      {
        id: 'terminal-safety',
        title: 'Protection against dangerous commands',
        badge: 'Guard',
        keywords: ['safety guard', 'dangerous command', 'production', 'rm -rf', 'confirm'],
        content: `### What it does
On servers marked as Staging or Production, VeloTerm pauses commands that could destroy the machine and asks you to confirm before they run.

### Where to find it
- Set it up in the host settings: choose environment "Staging" or "Production" and tick "Enable Safety Guard".

### How to use it
1. Protected terminals show warning stripes, and their tabs carry a red PROD badge.
2. Pressing Enter or pasting commands checks them first. The check sees through wrappers like \`sudo\` or \`nohup\`, but knows quoted text is harmless — \`echo "rm -rf /"\` is not blocked.
3. Extremely dangerous commands (e.g. \`rm -rf /\`) require typing a confirmation word; risky ones (e.g. reboot, \`kill -9 1\`) show a confirmation dialog. Clicking Cancel sends Ctrl+C and clears the line safely.

### Tips
- Dev and Test servers are not intercepted, so everyday work there stays fast.`,
      },
      {
        id: 'terminal-history',
        title: 'Command history',
        badge: 'History',
        keywords: ['command history', 'history', 'search', 'failed commands', 'rerun'],
        content: `### What it does
Keeps a record of the commands run on your servers — what was typed and whether it worked — so you can search, rerun or copy them later.

### Where to find it
- The "History" icon on the left sidebar.
- The "Install Guide" button inside the History screen.

### How to use it
1. Copy the one-time snippet from the Install Guide and paste it into \`~/.bashrc\`, \`~/.zshrc\` or your fish config on the server (or set it as the host's Ready Hook).
2. Work as usual — each command is recorded when it finishes.
3. In the History page: filter by server, search by text, show only failed commands, and rerun or copy any entry.

### Tips
- Recording can be turned off; either way, the behind-the-scenes reporting never leaves stray characters on your screen.`,
      },
      {
        id: 'terminal-interaction',
        title: 'Colors, links and drag-and-drop',
        badge: 'Basics',
        keywords: ['highlight', 'link', 'url', 'ip', 'drag and drop', 'paste', 'copy', 'search'],
        content: `### What it does
Makes terminal output easier to read: problems get colored automatically, web addresses and IP numbers are clickable, and dropped-in files arrive as ready-to-use paths.

### Where to find it
- Search in the terminal: \`⌘F\` (Mac) / \`Ctrl+F\` (Windows).
- Copy/paste: \`⌘C\` / \`⌘V\` on Mac; \`Ctrl+Shift+C\` / \`Ctrl+Shift+V\` on Windows (there, plain \`Ctrl+C\` copies when something is selected, plain \`Ctrl+V\` pastes).

### How to use it
1. Watch the colors: \`ERROR\` and \`FAILED\` red, \`WARN\` yellow, \`SUCCESS\` green.
2. Hold \`⌘\` (Mac) / \`Ctrl\` (Windows) and click a web address or IP to open it in your browser.
3. Drag a file from your desktop into the terminal — its path appears, correctly quoted for the shell you are using.

### Tips
- Search supports case-sensitive, whole-word and pattern (regex) matching; Enter / Shift+Enter steps through the hits.`,
      },
      {
        id: 'terminal-ai',
        title: 'AI helper for the terminal',
        badge: 'AI',
        keywords: ['ai', 'assistant', 'openai', 'deepseek', 'claude', 'gemini'],
        content: `### What it does
A built-in AI assistant that can look at what is on your terminal and suggest how to fix problems. Works with OpenAI, DeepSeek, Claude and Gemini.

### Where to find it
- The "AI" button in the terminal header, or \`⌘I\` (Mac) / \`Ctrl+I\` (Windows).

### How to use it
1. Under Settings → "AI Configuration", enter your API key and pick a provider (several keys and custom addresses are supported).
2. When something goes wrong, open the panel and describe the problem — it can look at the visible screen and logs.
3. Send a suggested command to the terminal with one click, or step through it under supervision.

### Tips
- The panel remembers its width, hides when you leave the terminal page and comes back when you return.`,
      },
    ],
  },
  {
    id: 'files',
    title: 'Files',
    icon: 'FolderSync',
    sections: [
      {
        id: 'files-manager',
        title: 'Two-sided file manager',
        badge: 'Files',
        keywords: ['files', 'sftp', 'transfer', 'drag', 'two panels', 'between servers'],
        content: `### What it does
A file manager with two panels side by side. Each side can show your own computer or a different server, so you can copy files between two servers by simply dragging.

### Where to find it
- "Files" on the left sidebar, or right-click a host card → "Open SFTP".

### How to use it
1. Connect one server on the left and another server (or your computer) on the right.
2. Use the path box to jump straight to a folder, or click the breadcrumbs to navigate.
3. Drag files from one side to the other — server-to-server copies travel directly, without detouring through your computer.

### Tips
- File owners and groups are shown as real names, not numbers.
- Leaving the Files page interrupts nothing: connections and ongoing transfers keep running in the background.`,
      },
      {
        id: 'files-transfer',
        title: 'Transfers: pause, resume and speed limits',
        badge: 'Queue',
        keywords: ['transfer queue', 'pause', 'resume', 'cancel', 'speed limit', 'progress'],
        content: `### What it does
One place to watch every copy in progress, with pause, resume, cancel and speed controls.

### Where to find it
- The "Transfer Queue" pill in the bar at the bottom of the file manager.

### How to use it
1. Glance at the footer for the overall speed and progress of current transfers.
2. Open the queue to pause, resume or cancel individual items.
3. Set the limits: how many transfers run at once, and the maximum upload/download speed.

### Tips
- Restarting an interrupted transfer picks up where it left off instead of starting over.`,
      },
      {
        id: 'files-dirs',
        title: 'Folder sizes, fast delete and terminal jumps',
        badge: 'Folders',
        keywords: ['calculate size', 'delete folder', 'fast delete', 'open in terminal'],
        content: `### What it does
Three time-savers in the right-click menu: measure how big a folder is, delete huge folders quickly, and jump straight into a folder in a terminal.

### Where to find it
- Right-click any file or folder in the Files list.

### How to use it
1. "Calculate Size" totals a folder's space and file count in the background — no waiting around.
2. "Delete" offers two modes: Shell fast delete (recommended — clears tens of thousands of files in seconds) or step-by-step delete (for servers that restrict shell commands).
3. "Open in Terminal" opens a new terminal tab already positioned in that folder.

### Tips
- Fast delete asks for one extra confirmation first, which protects important system folders.`,
      },
      {
        id: 'files-perms',
        title: 'Permissions and ownership',
        badge: 'Chmod',
        keywords: ['permissions', 'chmod', 'chown', 'owner', 'read write execute', '755'],
        content: `### What it does
Change who may read, change or run a file — and who owns it — by ticking boxes instead of memorizing numbers like 755.

### Where to find it
- Right-click a file or folder → "Properties / Permissions".

### How to use it
1. Tick the Owner, Group and Others boxes in the 3×3 grid; the number (e.g. \`755\`, \`644\`) updates alongside — or just type the number and the boxes follow.
2. Change the owner and group, if you have the rights.
3. Special flags (SUID, SGID, Sticky Bit) are available too.
4. When applying to a folder and everything inside it, choose "Smart Protection" to keep subfolders usable.

### Tips
- The dialog header shows handy file facts such as the last-modified time and the full path.`,
      },
      {
        id: 'files-sudo',
        title: 'Editing protected files with sudo',
        badge: 'Sudo',
        keywords: ['sudo', 'permission denied', 'protected file', 'root'],
        content: `### What it does
When a file answers "Permission denied" as you open or save it, VeloTerm can use your sudo password to read and write it anyway.

### Where to find it
- The prompt appears on its own when access is denied during reading or writing.

### How to use it
1. When denied, enter your sudo password (you can let VeloTerm remember it for the session).
2. The file opens for viewing, and saving writes it back safely.

### Tips
- This follows the server's normal sudo rules, so you can only do what your account is already allowed to do — and the server keeps its usual sudo records.`,
      },
      {
        id: 'files-diff',
        title: 'Comparing two files',
        badge: 'Diff',
        keywords: ['compare', 'diff', 'two files', 'side by side'],
        content: `### What it does
Shows exactly what differs between two files — side by side or as one continuous text — with live counts of added and removed lines.

### Where to find it
- Hold \`⌘\` (Mac) / \`Ctrl\` (Windows), click two files, then choose "Compare Files".
- Or right-click the first file → "Select for Compare", then right-click the second → "Compare with...".

### How to use it
1. The comparison window opens with one file on each side.
2. Switch between "Side-by-Side" and "Inline" views, or click "Swap Sides" to flip them.
3. Watch the live counters for added, deleted and identical lines.

### Tips
- The two files can live on different servers.`,
      },
    ],
  },
  {
    id: 'preview',
    title: 'Previewing files',
    icon: 'Eye',
    sections: [
      {
        id: 'preview-markdown',
        title: 'Markdown documents',
        badge: 'Markdown',
        keywords: ['markdown', 'md', 'readme', 'notes', 'checklist', 'table'],
        content: `### What it does
Shows .md files (like READMEs and notes) as tidy formatted pages — with headings, tables, checklists and colored code blocks.

### Where to find it
- Double-click or right-click any .md file in the Files list.

### How to use it
1. The file opens as a formatted page.
2. Code blocks carry a one-click copy button.
3. Switch freely between the formatted view, the plain source text, and a hex view.

### Tips
- Built for quick reading; to make changes, switch to the source view or open the file in the editor.`,
      },
      {
        id: 'preview-csv',
        title: 'CSV and TSV tables',
        badge: 'CSV',
        keywords: ['csv', 'tsv', 'table', 'sort', 'filter', 'excel', 'spreadsheet'],
        content: `### What it does
Opens .csv and .tsv files as a table you can sort, search and copy from — smoothly, even with tens of thousands of rows.

### Where to find it
- Double-click or right-click any .csv or .tsv file.

### How to use it
1. Click a column header to sort — numbers and text each sort correctly.
2. Type in the search box to narrow the table down to matching rows.
3. Select cells and copy; they paste cleanly into Excel or other spreadsheet apps.

### Tips
- The header row stays frozen at the top while you scroll.`,
      },
      {
        id: 'preview-json',
        title: 'JSON files',
        badge: 'JSON',
        keywords: ['json', 'jsonl', 'jq', 'tree', 'collapse', 'filter'],
        content: `### What it does
Shows .json and .jsonl files as a foldable, color-coded tree, with a built-in jq box for pulling out just the pieces you want.

### Where to find it
- Double-click or right-click any .json or .jsonl file.

### How to use it
1. Click the arrows to fold and unfold; the toolbar offers "Expand All" / "Collapse All".
2. Hover over any item for one-click copying of its path, name or value.
3. Type a jq filter such as \`.data[] | {id, name}\` in the top box and press Enter — the tree rebuilds showing only the matches.

### Tips
- jq filters run with a short safety timeout, so a mistaken filter can't freeze the view.`,
      },
      {
        id: 'preview-log',
        title: 'Big log files',
        badge: 'Log',
        keywords: ['log', 'tail', 'follow', 'large file', 'error log'],
        content: `### What it does
Opens even multi-gigabyte log files instantly by loading the newest part first — and can follow along as new lines arrive.

### Where to find it
- Double-click or preview any .log, .out or .err file.

### How to use it
1. The end of the file opens immediately.
2. Switch on live following (like \`tail -f\`) to watch new lines appear automatically.
3. Filter by level (ALL / ERROR / WARN / INFO); "Load Earlier Logs" pages back through older lines.

### Tips
- This skips the usual size limit for file previews, so you can check huge production logs right away.`,
      },
      {
        id: 'preview-hex',
        title: 'Hex view for other files',
        badge: 'Hex',
        keywords: ['hex', 'binary', 'bytes', 'xxd'],
        content: `### What it does
Shows a file's raw contents as rows of hexadecimal numbers — for files that aren't plain text, like programs, libraries and data files.

### Where to find it
- Double-click binary files (.so, .bin, .exe, .db and the like), or switch to the Hex view inside other previews.

### How to use it
1. Read the three columns: position, hex values, and printable characters.
2. Click a byte — the bottom bar shows its value as Hex, Dec, Bin and Char, and the matching character lights up on the right.
3. One click exports a standard xxd-style text dump.

### Tips
- Files of any size open comfortably; only the visible part is ever loaded.`,
      },
      {
        id: 'preview-excel',
        title: 'Excel spreadsheets',
        badge: 'Excel',
        keywords: ['excel', 'xlsx', 'xls', 'xlsb', 'ods', 'sheet'],
        content: `### What it does
Opens Excel files (.xlsx, .xls, .xlsb, .ods) as tables — without Microsoft Office and without internet.

### Where to find it
- Double-click or preview any of those files in the Files list.

### How to use it
1. Sheets appear as tabs along the bottom; click to switch between them.
2. Sort columns, search to filter rows, and copy cells out.

### Tips
- Great for a quick look at a spreadsheet that lives on a server — it opens right in place.`,
      },
      {
        id: 'preview-sqlite',
        title: 'SQLite database files',
        badge: 'SQLite',
        keywords: ['sqlite', 'db', 'database file', 'sql', 'select'],
        content: `### What it does
Opens .db, .sqlite and .sqlite3 files so you can browse their tables and run look-up queries — strictly read-only, so the file can't be damaged.

### Where to find it
- Double-click or preview any SQLite database file.

### How to use it
1. Tables and views are listed on the left, each with its row count.
2. Click a table to browse its contents.
3. Type a query (SELECT, PRAGMA, EXPLAIN) in the top box and run it.

### Tips
- Changing commands (INSERT, UPDATE, DROP and friends) are refused before they ever reach the file.`,
      },
      {
        id: 'preview-cert',
        title: 'Certificates',
        badge: 'Cert',
        keywords: ['certificate', 'ssl', 'https', 'expiry', 'domain', 'pem'],
        content: `### What it does
Opens certificate files (.crt, .cer, .pem, .ca-bundle) and tells you at a glance whether they are still valid, who issued them, and which domains they cover.

### Where to find it
- Double-click or preview any of those files.

### How to use it
1. A colored badge shows the state — green valid, yellow expiring within 30 days, red expired — along with the days remaining.
2. Look up subject, issuer, validity dates, serial number, signature and key details; bundle files let you step through the whole chain.
3. Copy the full list of covered domains with one click; SHA-256 and SHA-1 fingerprints are shown too.

### Tips
- Handy for catching a certificate that is about to expire before it causes an outage.`,
      },
      {
        id: 'preview-media',
        title: 'Pictures and PDFs',
        badge: 'Media',
        keywords: ['image', 'picture', 'png', 'jpg', 'svg', 'webp', 'jxl', 'pdf'],
        content: `### What it does
View pictures — PNG, JPG, SVG, WebP and the newer space-saving .jxl — and PDF documents right inside VeloTerm.

### Where to find it
- Double-click any supported picture or .pdf file.

### How to use it
1. Pictures: zoom and pan freely; dimensions are displayed.
2. PDFs: page through and zoom; text stays crisp at any size.

### Tips
- Everything needed is built in — nothing to install, and it works offline.`,
      },
    ],
  },
  {
    id: 'editor',
    title: 'Editor',
    icon: 'FileCode2',
    sections: [
      {
        id: 'editor-core',
        title: 'The built-in editor',
        badge: 'Core',
        keywords: ['editor', 'edit', 'save', 'colors', 'offline', 'vs code'],
        content: `### What it does
A comfortable text editor for code and settings files — the same editor used by VS Code — with coloring for 81 languages, working fully offline.

### Where to find it
- Right-click a file → "Open in Editor", or the "Editor" icon on the sidebar.
- Save: \`⌘S\` (Mac) / \`Ctrl+S\` (Windows).

### How to use it
1. Open files; each gets its own tab.
2. Enjoy the helpers: collapsible sections, matching brackets shown in color, indent guides, and Alt+Click to place several cursors at once.
3. Press \`⌘S\` / \`Ctrl+S\` to save back to the server — a message confirms it worked.

### Tips
- Everything is built in, so it also works on networks with no internet access.`,
      },
      {
        id: 'editor-nginx',
        title: 'Nginx configuration files',
        badge: 'Nginx',
        keywords: ['nginx', 'conf', 'server', 'location', 'variables', 'suggestions'],
        content: `### What it does
When you edit an nginx configuration, the editor suggests entries that are valid exactly where you are typing, explains them on hover, and completes variable names.

### Where to find it
- Just open an nginx file — \`nginx.conf\`, \`.conf\` files under \`/nginx/\`, or similar configs.

### How to use it
1. Start typing to get suggestions that fit the current section (e.g. \`proxy_pass\` inside \`location\`).
2. Type \`$\` to list built-in variables like \`$host\` and \`$remote_addr\`.
3. Rest the mouse on an entry to see its meaning and default value.

### Tips
- Files without the usual big blocks — like \`mime.types\` or \`fastcgi_params\` — are recognized too.`,
      },
      {
        id: 'editor-docker',
        title: 'Dockerfile and Compose files',
        badge: 'Docker',
        keywords: ['docker', 'dockerfile', 'compose', 'yaml', 'buildkit'],
        content: `### What it does
While you edit Dockerfile or Docker Compose files, the editor suggests the right instructions and names, so you don't have to remember them.

### Where to find it
- Open a file named \`Dockerfile\`, \`docker-compose.yml\` or \`compose.yaml\`.

### How to use it
1. Dockerfile: suggestions cover all official instructions and options; \`COPY --from=\` even offers the names of earlier build stages.
2. Type \`$\` or \`\${\` to get the ARG/ENV variables you defined earlier.
3. Compose files: keys are suggested as you indent, and service names appear under \`depends_on\`, \`networks\` and friends.

### Tips
- Ordinary YAML files (like CI configs) are left alone.`,
      },
      {
        id: 'editor-bash',
        title: 'Shell scripts',
        badge: 'Shell',
        keywords: ['bash', 'shell', 'sh', 'zsh', 'script', 'variables', 'functions'],
        content: `### What it does
Editing help for shell scripts: suggests keywords, the functions and variables your script defines — and, for files on a server, the commands that server actually has.

### Where to find it
- Open any \`.sh\`, \`.bash\` or \`.zsh\` file.

### How to use it
1. Type to get standard keywords and common built-in commands.
2. Suggestions include the functions and variables defined in your script.
3. For files on a server, that server's own command list joins the suggestions automatically.

### Tips
- If the server can't be asked, editing continues normally — you simply see fewer suggestions.`,
      },
      {
        id: 'editor-batch',
        title: 'Windows batch files',
        badge: 'Batch',
        keywords: ['batch', 'bat', 'cmd', 'goto', 'call', 'label'],
        content: `### What it does
Editing help for Windows .bat and .cmd scripts: clear colors, command suggestions, and jumps to \`goto\` labels.

### Where to find it
- Open any \`.bat\` or \`.cmd\` file.

### How to use it
1. Comments, labels and variables like \`%var%\` and \`%%i\` each get their own color.
2. Get suggestions for Windows commands and ready-made snippets.
3. Right-click a \`goto label\` or \`call :label\` → "Go to Definition" (\`F12\`) jumps to the label; labels can also be renamed everywhere at once.

### Tips
- Works fully offline.`,
      },
      {
        id: 'editor-sql',
        title: 'SQL files',
        badge: 'SQL',
        keywords: ['sql', 'mysql', 'postgresql', 'sqlite', 'keywords', 'functions'],
        content: `### What it does
SQL files get keyword and function suggestions — standard SQL plus extras for MySQL 8, PostgreSQL 13 and SQLite 3.

### Where to find it
- Open any \`.sql\` file.

### How to use it
1. Type to get keywords in capitals (\`SELECT\`, \`WHERE\`, \`GROUP BY\`...).
2. Nearly 500 function names are included, alongside dialect keywords.
3. Snippets like \`create_table\` expand into a ready-made skeleton.

### Tips
- Suggestions stay sensible — function names don't pop up right after FROM or JOIN, where a table belongs.`,
      },
      {
        id: 'editor-systemd',
        title: 'Linux service files',
        badge: 'Systemd',
        keywords: ['systemd', 'service', 'socket', 'timer', 'execstart', 'restart'],
        content: `### What it does
Editing help for Linux service configurations (\`.service\`, \`.socket\`, \`.timer\`, \`.target\` and more): suggests valid settings for each section, and the allowed values.

### Where to find it
- Open one of those files in the editor.

### How to use it
1. Section headers like \`[Unit]\` and \`[Service]\` get their own colors.
2. Inside a section, only settings that make sense there are suggested — \`ExecStart\`, \`Restart\`, \`User\` in \`[Service]\`, for example.
3. After typing \`=\` you are offered the valid choices (\`Restart=\` suggests \`always\`, \`on-failure\`, \`no\`...).

### Tips
- Rest the mouse on a setting for a one-line explanation from the official documentation.`,
      },
      {
        id: 'editor-crontab',
        title: 'Scheduled tasks (crontab)',
        badge: 'Cron',
        keywords: ['crontab', 'cron', 'schedule', 'next run', 'validation'],
        content: `### What it does
Checks scheduled-task timing lines as you type, and shows when the task will next run — no online cron calculator needed.

### Where to find it
- Open a file named \`crontab\` (or with the .crontab ending).

### How to use it
1. Mistakes — like a minute value above 59 — are underlined in red on the spot.
2. Hover over a valid timing line to see the next 3 run times, in your local time.
3. All the usual forms work: ranges, steps, month/day names, and shortcuts like \`@daily\` and \`@reboot\`.

### Tips
- Ready-made schedule templates are offered as snippets.`,
      },
      {
        id: 'editor-html',
        title: 'HTML and CSS files',
        badge: 'HTML',
        keywords: ['html', 'css', 'tags', 'auto close', 'quotes', 'scss'],
        content: `### What it does
Help while writing web pages: tags close themselves, quotes appear in pairs, and full suggestions for tags and attributes are built in.

### Where to find it
- Open \`.html\`, \`.htm\` or \`.css\` files (SCSS and LESS too).

### How to use it
1. Typing \`>\` after \`<div>\` adds the closing \`</div>\` for you.
2. Typing \`</\` completes the nearest open tag.
3. Typing \`=\` after an attribute inserts the quotes and places the cursor between them.

### Tips
- The suggestions match what VS Code would offer — and they work offline.`,
      },
      {
        id: 'editor-draft',
        title: 'Drafts and conflict protection',
        badge: 'Safety',
        keywords: ['draft', 'unsaved', 'recovery', 'conflict', 'overwrite', 'crash'],
        content: `### What it does
Your typing is saved as you go, so a crash or an accidental close loses nothing. And if someone else changed the file on the server in the meantime, you are asked before your version overwrites theirs.

### Where to find it
- Nothing to open — it watches quietly while you edit.

### How to use it
1. Reopen a file after a crash or accidental close; your unsaved changes are offered back to you.
2. On save, if the server's copy has changed, choose "Overwrite Remote" (keep yours) or "Discard Draft" (take theirs).

### Tips
- Before deciding, you can view the two versions side by side.`,
      },
    ],
  },
  {
    id: 'network',
    title: 'Connections & network tools',
    icon: 'Network',
    sections: [
      {
        id: 'network-relay',
        title: 'Relay servers',
        badge: 'Relay',
        keywords: ['relay', 'tunnel', 'bastion', 'gateway'],
        content: `### What it does
When servers are only reachable through a special gateway machine, a relay connection lets VeloTerm reach them — terminals, file transfers and port forwarding all work as usual through it.

### Where to find it
- "Relays" on the left sidebar, to manage them.
- Per server: in host settings, tick "Connect via Relay" and pick one.

### How to use it
1. Click "New Relay" and fill in the address, port, connection type (TCP / WS / QUIC) and the access credentials (Key ID + Secret).
2. Click "Test Connection" — it checks the relay and shows the response time in milliseconds.
3. Assign the relay to a server; its connections then travel through it.

### Tips
- Your secret is never sent over the connection, so it can't be overheard.`,
      },
      {
        id: 'network-topology',
        title: 'Connection map (topology)',
        badge: 'Topology',
        keywords: ['topology', 'map', 'overview', 'chain', 'status'],
        content: `### What it does
Draws a picture of how you are connected: your computer → relay → jump hosts → target server → port forwards, with live status for each step.

### Where to find it
- Single server: the "Topology" network icon on its host card.
- Everything at once: "Global Topology" in the toolbar of the Hosts page.

### How to use it
1. The single-server view shows every step in the chain and its state (Idle, Connecting, Active, Error).
2. The global view puts all your servers on one canvas, with the active ones pinned at the top.
3. Click any node to see its status or start a session from there.

### Tips
- Port forwarding rules can be switched on and off right inside the map.`,
      },
      {
        id: 'network-forward',
        title: 'Port forwarding',
        badge: 'Port',
        keywords: ['port forwarding', 'tunnel', 'localhost', 'remote service'],
        content: `### What it does
Lets you use a service that runs on a server (or behind it) as if it were running on your own computer — for example, opening a server's admin page in your browser at \`localhost\`.

### Where to find it
- The "Port Forward" icon on a host card, or inside the topology view.

### How to use it
1. Click "Add Rule": pick a local port (e.g. \`8080\`) and the destination (e.g. \`10.0.0.5:80\`).
2. Let rules start automatically with the session, or switch them on only when needed.
3. The rule list shows running time and warns you about port conflicts.

### Tips
- Combines with jump hosts and relays, so even deeply buried servers are reachable.`,
      },
      {
        id: 'network-zk',
        title: 'ZooKeeper browser',
        badge: 'ZK',
        keywords: ['zookeeper', 'zk', 'znode', 'dubbo', 'acl'],
        content: `### What it does
Browse and edit a ZooKeeper cluster's data tree: inspect nodes, change their data, manage access rights, and create or delete nodes.

### Where to find it
- "ZK Hosts" on the sidebar to save cluster addresses; the "ZooKeeper" page to browse them.

### How to use it
1. Expand the tree, or search by keyword.
2. Click a node to view and edit its data (as text or hex) and check its details — creation and modification times, version and more.
3. Create nodes (normal, temporary, numbered) or delete a single node or a whole branch; access rights can be edited too.
4. Login credentials for protected clusters can be saved in the credentials manager.

### Tips
- Nodes holding Dubbo addresses offer an "Open in gRPC" shortcut.`,
      },
      {
        id: 'network-grpc',
        title: 'gRPC testing',
        badge: 'gRPC',
        keywords: ['grpc', 'api', 'test', 'tls', 'reflection'],
        content: `### What it does
Try out a gRPC service without writing a program: see what it offers, fill in a prepared request, and read the answer.

### Where to find it
- "gRPC" on the sidebar, or "Open in gRPC" from a ZooKeeper Dubbo node.

### How to use it
1. Enter the service address (e.g. \`127.0.0.1:50051\`) and click "Fetch Services" to list what is available.
2. Pick a method — a sample request appears, ready for you to edit.
3. Click "Invoke" to see the response, the timing and the status.

### Tips
- Secure connections are supported, including client certificates; your entries are remembered when you leave the page.`,
      },
    ],
  },
  {
    id: 'dbworkbench',
    title: 'Database',
    icon: 'Database',
    sections: [
      {
        id: 'db-connections',
        title: 'Saving database connections',
        badge: 'DB',
        keywords: ['database', 'mysql', 'postgresql', 'oracle', 'dameng', 'dm8', 'connection', 'ssl', 'read-only', 'compatibility mode'],
        content: `### What it does
Save connections to MySQL, PostgreSQL, Oracle and Dameng DM8 databases, with passwords stored encrypted, a protective read-only mode, and a one-click connection test.

### Where to find it
- The "DB Hosts" page on the sidebar.

### How to use it
1. Click "New Connection" and pick the database type — the usual port is filled in for you (MySQL 3306, PostgreSQL 5432, Oracle 1521, Dameng 5236).
2. Enter the address, database, user name and password; pick an SSL option if the server supports encrypted connections (Dameng doesn't offer one, so the choice is hidden there).
3. Optionally set the environment (dev/test/staging/prod) and keep "Read-only session" switched on — changes are then refused.
4. Click "Test" before saving: it connects, reports the server version and the round-trip time in milliseconds.

### Tips
- Editing a saved connection with the password box left empty keeps the old password.
- Dameng specifics: the MySQL/Oracle "Compatibility" choice only changes how VeloTerm handles its SQL — it never affects whether you can connect; the database box may be left empty; and no extra driver software is needed.`,
      },
      {
        id: 'db-workbench',
        title: 'Writing and running SQL',
        badge: 'SQL',
        keywords: ['sql', 'query', 'tables', 'run', 'cancel', 'results', 'workbench'],
        content: `### What it does
A workspace for SQL: browse the database's tables and objects on the left, write queries on the right, and read results in tidy grids.

### Where to find it
- Press ▶ on a connection in "DB Hosts", or click + in the workspace and pick a saved connection.
- Run: \`⌘/Ctrl + Enter\`; the cancel button and the row limit (1,000–50,000) sit on the toolbar.

### How to use it
1. Expand a database to find its tables, views, functions and more; click a table to see its columns, or double-click it to get a ready-made \`SELECT\` in the editor.
2. Write one or more statements (separate them with semicolons) and run with \`⌘/Ctrl + Enter\`.
3. Each statement gets its own result tab — sortable, filterable, copyable. Long queries can be stopped at any moment.

### Tips
- Switching pages keeps everything as it was; closing a tab disconnects. In read-only connections, changes are stopped with a clear explanation.`,
      },
    ],
  },
  {
    id: 'hosts',
    title: 'Hosts & groups',
    icon: 'Server',
    sections: [
      {
        id: 'hosts-manage',
        title: 'Organizing your servers',
        badge: 'Asset',
        keywords: ['hosts', 'servers', 'groups', 'folders', 'prod', 'environment', 'tags'],
        content: `### What it does
Keeps all your servers in one tidy list — folders, tags, colors — and lets you mark which ones are production so they get extra protection.

### Where to find it
- "Hosts" and "Groups" on the sidebar; the search box filters by name, IP, tag or user name.

### How to use it
1. Build a folder tree in "Groups" — by project, region, or however you like.
2. Add servers in "Hosts": name, address, port, user name and sign-in.
3. Tag each server's environment as Dev, Test, Staging or Prod — Prod turns on warning stripes and extra confirmations.
4. Right-click a host to copy its connection details or create a quick-launch shortcut.

### Tips
- Give hosts their own colors so several open terminals are easy to tell apart.`,
      },
      {
        id: 'hosts-password-history',
        title: 'Password history',
        badge: 'Backup',
        keywords: ['password', 'history', 'old password', 'restore', 'forgot'],
        content: `### What it does
Every time a server's password changes, the previous one is kept (up to 20 of them), so an old password is never lost for good.

### Where to find it
- The clock icon next to the password box in the host edit form.

### How to use it
1. Click the clock icon to open the history list — past dates with masked passwords.
2. Copy one to the clipboard, or click "Restore This Password" to bring it back.

### Tips
- The list stays on your own computer, stored encrypted.`,
      },
      {
        id: 'hosts-ready-hook',
        title: 'Running a command right after connecting',
        badge: 'Hook',
        keywords: ['ready hook', 'auto command', 'after connect', 'startup'],
        content: `### What it does
VeloTerm can type a command for you automatically each time a connection becomes ready — for example jumping into the project folder or checking a service's status.

### Where to find it
- Host settings → "Advanced Options" → "Ready Hook".

### How to use it
1. Pick one of the commands from your Command Library.
2. Save the host and connect — the command runs by itself once the welcome text has settled.

### Tips
- It waits for the screen to go quiet first, so the command isn't swallowed by the server's welcome banner.`,
      },
    ],
  },
  {
    id: 'commands',
    title: 'Command library',
    icon: 'TerminalSquare',
    sections: [
      {
        id: 'commands-library',
        title: 'Built-in command library',
        badge: 'Library',
        keywords: ['commands', 'library', 'templates', 'snippets', 'run', 'placeholders'],
        content: `### What it does
A searchable collection of 192 ready-made commands — Linux and Windows — each explained in plain words. You can add your own, with fill-in-the-blank spots.

### Where to find it
- "Commands" on the sidebar.

### How to use it
1. Browse or search, then click "Run" on a card.
2. A small form asks for the blanks — e.g. \`docker logs -n {{lines}} {{container}}\` — fill them in.
3. Send the command to the terminal, or run it in the background. Your collection can be exported to a file and re-imported elsewhere.

### Tips
- Built-in entries carry verified sources and safety flags, warning you before a destructive command runs.`,
      },
      {
        id: 'commands-palette',
        title: 'Quick command search',
        badge: 'Palette',
        keywords: ['command palette', 'quick search', 'launcher', '⌘k'],
        content: `### What it does
A search box that finds and runs any command from anywhere in the app — no clicking through pages needed.

### Where to find it
- \`⌘K\` (Mac) / \`Ctrl+K\` (Windows), anywhere in the app.

### How to use it
1. Press the shortcut and type a few words — titles and command text are searched.
2. Move with the arrow keys, press Enter to send to the terminal, Esc to close.

### Tips
- Fill-in blanks can be completed right inside the search window.`,
      },
    ],
  },
  {
    id: 'tools',
    title: 'Built-in tools (Windows only)',
    icon: 'Wrench',
    sections: [
      {
        id: 'tools-coreutils',
        title: 'Linux commands built into Windows',
        badge: 'Coreutils',
        keywords: ['windows', 'linux commands', 'ls', 'grep', 'sed', 'awk', 'curl'],
        content: `### What it does
> Windows only.
On Windows, VeloTerm brings 130+ familiar Linux commands along — \`ls\`, \`cat\`, \`grep\`, \`sed\`, \`find\`, \`tar\`, \`curl\`, \`tree\`, \`ps\`, \`top\` and many more — with nothing to install.

### Where to find it
- Just open a local terminal and type them.

### How to use it
1. Open a local terminal (the "+" button or the new-terminal shortcut).
2. Type any of the built-in commands exactly as you would on a Linux server — for listing, searching, packing, downloading and more.

### Tips
- No Git Bash, Cygwin or anything else is needed — it all works out of the box.`,
      },
      {
        id: 'tools-enhanced',
        title: 'Extra tools: search, JSON, vim and more',
        badge: 'Pro',
        keywords: ['ripgrep', 'rg', 'fzf', 'jq', 'yq', 'nmap', 'vim', 'bc', 'qalc', 'openssl', 'zoxide'],
        content: `### What it does
> Windows only.
Beyond the basics, favorite power tools are included: \`rg\` for lightning-fast content search, \`fzf\` as an interactive finder, \`jq\` and \`yq\` for JSON and YAML, \`nmap\`/\`ncat\`/\`nping\` for network checks, the complete \`openssl\` toolkit, \`vim\`, the calculators \`bc\` and \`qalc\`, and \`zoxide\` for jumping to folders you visit often.

### Where to find it
- Run them by name in a local terminal.

### How to use it
1. Open a local terminal.
2. Run any tool by name — for example \`vim <file>\` edits a file right inside the terminal.

### Tips
- All of them are tuned to display text cleanly, without garbled characters.`,
      },
      {
        id: 'tools-hardlinks',
        title: 'Automatic setup of the built-in commands',
        badge: 'Auto',
        keywords: ['automatic', 'tools folder', 'setup', 'short names'],
        content: `### What it does
> Windows only.
VeloTerm prepares every built-in command by itself when it starts, so they are all ready to use by their short names — zero setup for you.

### Where to find it
- Nothing to find — it happens automatically at startup.

### How to use it
1. Open a local terminal and type any command name (\`ls\`, \`grep\`, ...) — it simply works.
2. After a VeloTerm upgrade, the command set refreshes itself.

### Tips
- The commands live in the app's \`tools\` folder; you never need to touch it.`,
      },
    ],
  },
  {
    id: 'settings',
    title: 'Settings & security',
    icon: 'ShieldCheck',
    sections: [
      {
        id: 'settings-theme',
        title: 'Appearance',
        badge: 'Theme',
        keywords: ['theme', 'dark', 'light', 'font size', 'colors', 'terminal colors'],
        content: `### What it does
Switch between dark and light looks, and adjust how large text is across the whole app.

### Where to find it
- The gear icon at the bottom-left, or \`⌘,\` (Mac) / \`Ctrl+,\` (Windows), then "Appearance".

### How to use it
1. Choose Dark, Light, or follow the System setting.
2. Adjust the font size — menus, cards and forms all scale with it.
3. Terminal colors can be customized per server, and recordings replay with those same colors.

### Tips
- Everything changes instantly — no restart needed.`,
      },
      {
        id: 'settings-sidebar',
        title: 'Customizing the sidebar',
        badge: 'Sidebar',
        keywords: ['sidebar', 'menu', 'reorder', 'hide', 'collapse', 'width'],
        content: `### What it does
Hide the icons you never use, put the rest in your favorite order, and resize the sidebar to taste.

### Where to find it
- Collapse/expand: the toggle at the top-left, or \`⌘B\` (Mac) / \`Ctrl+B\` (Windows).
- Customize: Settings → "Sidebar Visibility".

### How to use it
1. Tick or untick pages (gRPC, ZooKeeper, Relays, ...) to show or hide them.
2. Drag rows up and down to reorder — changes are saved immediately.
3. Drag the sidebar's right edge to change its width.

### Tips
- The Settings icon always stays pinned at the bottom, so you can't lock yourself out.`,
      },
      {
        id: 'settings-logging',
        title: 'Logs for troubleshooting',
        badge: 'Log',
        keywords: ['log', 'log level', 'debug', 'troubleshooting', 'log file'],
        content: `### What it does
For troubleshooting, VeloTerm can write more detailed log files and show you exactly where they are on your computer.

### Where to find it
- Settings → "Diagnostics & Logs".

### How to use it
1. Pick a detail level from the dropdown — Info for everyday, Debug or Trace when hunting a problem.
2. Read the log file's location, or click "Reveal in File Manager" to jump straight to the folder.
3. Level changes take effect immediately; no restart.

### Tips
- Log lines carry timestamps and topic labels, which makes them easy to share with support.`,
      },
      {
        id: 'settings-known-hosts',
        title: 'Trusted servers',
        badge: 'Security',
        keywords: ['known hosts', 'fingerprint', 'security', 'revoke', 'first connection'],
        content: `### What it does
The list of servers you have told VeloTerm to trust. It protects you if a server's identity suddenly changes, and lets you tidy up after a server has been rebuilt.

### Where to find it
- Settings → "Security & Trust".

### How to use it
1. Review the saved servers: address, key type (e.g. \`ssh-ed25519\`) and SHA-256 fingerprint.
2. After a server is reinstalled, find its entry and click "Revoke" to clear the outdated fingerprint.
3. "Clear All Fingerprints" empties the whole list if you are troubleshooting connections.

### Tips
- Connecting to a server that isn't on the list yet always asks you to confirm first.`,
      },
      {
        id: 'settings-encryption',
        title: 'Your data is stored encrypted',
        badge: 'Safe',
        keywords: ['encryption', 'security', 'passwords', 'data protection', 'window layout'],
        content: `### What it does
Passwords, private keys, drafts, history and even your window size and position are saved on your computer in strongly encrypted form — readable only by VeloTerm.

### Where to find it
- Always on — there is nothing to set up.

### How to use it
1. Everything sensitive is scrambled (AES-256) before it is written to disk.
2. Quit and reopen VeloTerm: your window reappears exactly where and how you left it.

### Tips
- Even someone who copied the app's data files couldn't read your passwords from them.`,
      },
    ],
  },
];
