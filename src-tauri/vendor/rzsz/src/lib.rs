//! VeloTerm vendor 子集：ZMODEM 协议引擎 + 可移植 reader（serial/reader.rs）。
//! 裁掉：tty/stdio 胶水（termios/ProtocolWriter）、xmodem/ymodem、zz 二进制——见 README.md。
pub mod zmodem;
pub mod serial;
pub mod sender;
pub mod receiver;
