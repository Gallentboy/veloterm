/**
 * Windows Batch 命令关键字表。
 *
 * BATCH_COMMANDS（316 条）逐条取自 rech-editor-batch 0.0.26 的
 * syntaxes/batchfile.tmLanguage.json commands 规则的 match 正则（MIT，来源
 * 与升级指引见本目录 README.md）。含少量上游笔误词条（brea/hep/ntackup/
 * ysocmgr 等）与多词条目（net computer 等），保真保留：与上游 diff 零噪音，
 * 高亮无害，补全端 Monaco 前缀过滤自然吞掉。
 *
 * BATCH_KEYWORDS 是 cmd 内建控制流/内建命令（if/for/goto/set/echo 等，
 * 上游 grammar 的 controls 规则 + setlocal 专项规则合并）——上游命令表
 * 只收 Windows 系统工具（外置命令），内建不在其中，补全需要单列。
 */

/** Windows 系统命令（上游 grammar commands 正则的 alternation 全集，去重） */
export const BATCH_COMMANDS: readonly string[] = [
  'adprep', 'append', 'arp', 'assoc', 'at', 'atmadm', 'attrib', 'auditpol', 'autochk',
  'autoconv', 'autofmt', 'bcdboot', 'bcdedit', 'bdehdcfg', 'bitsadmin', 'bootcfg', 'brea',
  'cacls', 'cd', 'certreq', 'certutil', 'change', 'chcp', 'chdir', 'chglogon', 'chgport',
  'chgusr', 'chkdsk', 'chkntfs', 'choice', 'cipher', 'clip', 'cls', 'clscluadmin', 'cluster',
  'cmd', 'cmdkey', 'cmstp', 'color', 'comp', 'compact', 'convert', 'copy', 'cprofile',
  'cscript', 'csvde', 'date', 'dcdiag', 'dcgpofix', 'dcpromo', 'defra', 'del', 'dfscmd',
  'dfsdiag', 'dfsrmig', 'diantz', 'dir', 'dirquota', 'diskcomp', 'diskcopy', 'diskpart',
  'diskperf', 'diskraid', 'diskshadow', 'dispdiag', 'dnscmd', 'doin', 'doskey', 'driverquery',
  'dsacls', 'dsadd', 'dsamain', 'dsdbutil', 'dsget', 'dsmgmt', 'dsmod', 'dsmove', 'dsquery',
  'dsrm', 'edit', 'endlocal', 'eraseesentutl', 'eventcreate', 'eventquery', 'eventtriggers',
  'evntcmd', 'expand', 'extract', 'fc', 'filescrn', 'find', 'findstr', 'finger', 'flattemp',
  'fonde', 'forfiles', 'format', 'freedisk', 'fsutil', 'ftp', 'ftype', 'fveupdate', 'getmac',
  'gettype', 'gpfixup', 'gpresult', 'gpupdate', 'graftabl', 'hashgen', 'helpctr', 'hep',
  'hostname', 'icacls', 'iisreset', 'inuse', 'ipconfig', 'ipxroute', 'irftp', 'ismserv',
  'jetpack', 'klist', 'ksetup', 'ktmutil', 'ktpass', 'label', 'ldifd', 'ldp', 'lodctr',
  'logman', 'logoff', 'lpq', 'lpr', 'macfile', 'makecab', 'manage-bde', 'mapadmin', 'md',
  'mic', 'mkdir', 'mklink', 'mmc', 'mode', 'more', 'mount', 'mountvol', 'move', 'mqbup',
  'mqsvc', 'mqtgsvc', 'msdt', 'msg', 'msiexec', 'msinfo32', 'mstsc', 'nbtstat', 'net',
  'net computer', 'net group', 'net localgroup', 'net print', 'net session', 'net share',
  'net start', 'net stop', 'net use', 'net user', 'net view', 'netcfg', 'netdiag', 'netdom',
  'netsh', 'netstat', 'nfsadmin', 'nfsshare', 'nfsstat', 'nlb', 'nlbmgr', 'nltest', 'nslookup',
  'ntackup', 'ntcmdprompt', 'ntdsutil', 'ntfrsutl', 'openfiles', 'pagefileconfig', 'path',
  'pathping', 'pause', 'pbadmin', 'pentnt', 'perfmon', 'ping', 'pnpunatten', 'pnputil', 'popd',
  'powercfg', 'powershell', 'powershell_ise', 'print', 'prncnfg', 'prndrvr', 'prnjobs',
  'prnmngr', 'prnport', 'prnqctl', 'prompt', 'pubprn', 'pushd', 'pushprinterconnections',
  'pwlauncher', 'qappsrv', 'qprocess', 'query', 'quser', 'qwinsta', 'rasdial', 'rcp', 'rd',
  'rdpsign', 'recover', 'redircmp', 'redirusr', 'reg', 'regentc', 'regini', 'regsvr32',
  'relog', 'ren', 'rename', 'rendom', 'repadmin', 'repair-bde', 'replace', 'reset session',
  'risetup', 'rmdir', 'robocopy', 'route', 'rpcinfo', 'rpcping', 'rsh', 'runas', 'rundll32',
  'rwinsta', 'rxec', 'sc', 'schtasks', 'scp', 'scwcmd', 'secedit', 'serverceipoptin',
  'serverweroptin', 'servrmanagercmd', 'setspn', 'setx', 'sfc', 'shadow', 'shift', 'showmount',
  'shutdown', 'sort', 'ssh', 'start', 'storrept', 'subst', 'sxstrace', 'systeminfo', 'tacerpt',
  'takeown', 'tapicfg', 'taskkill', 'tasklist', 'tcmsetup', 'telnet', 'tftp', 'time',
  'timeout', 'title', 'tlntadmn', 'tpmvscmgr', 'tracert', 'tree', 'tscon', 'tsdiscon',
  'tsecimp', 'tskill', 'tsprof', 'type', 'typeperf', 'tzutil', 'uddiconfig', 'umount',
  'unlodctr', 'ver', 'verif', 'verifier', 'vol', 'vssadmin', 'w32tm', 'waitfor', 'wbadmin',
  'wdsutil', 'wecutil', 'wevtutil', 'where', 'whoami', 'winnt', 'winnt32', 'winpop', 'winrm',
  'winrs', 'winsat', 'wlbs', 'wscript', 'xcopy', 'ysocmgr',
];

/** cmd 内建控制流与内建命令（上游 controls/setlocal 规则 + 内建全集，外置命令表之外的补集） */
export const BATCH_KEYWORDS: readonly string[] = [
  'call',
  'cls',
  'cmdextversion',
  'defined',
  'do',
  'echo',
  'else',
  'errorlevel',
  'exit',
  'exist',
  'for',
  'goto',
  'if',
  'in',
  'not',
  'off',
  'on',
  'pause',
  'rem',
  'set',
  'setlocal',
];
