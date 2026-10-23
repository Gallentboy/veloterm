#!/usr/bin/env python3
"""生成 Rust 交叉验证向量（tests/vectors.rs）。

用 Python 参考实现（/var/tmp/dm_pure）生成随机输入 + 期望输出，
Rust 侧 tests/cross_validate.rs 逐条比对，验证位压缩/DECIMAL 编解码与 Python 完全一致。

用法: python3 scripts/gen_vectors.py > tests/vectors.rs
"""
import sys
import random
import datetime

sys.path.insert(0, "/var/tmp/dm_pure")
from dmpy.client import _enc_date, _enc_time, _enc_datetime
from dmpy.result import _decode_decimal


def main(seed: int = 42, count: int = 120):
    random.seed(seed)
    out = []
    out.append("// 由 scripts/gen_vectors.py 生成的交叉验证向量（随机种子 42）")
    out.append("// 与 Python dmpy 参考实现逐条比对")

    out.append("pub const DATE_VECTORS: &[(&str, i32, u8, u8)] = &[")
    for _ in range(count):
        y = random.randint(1, 9999)
        m = random.randint(1, 12)
        d = random.randint(1, 28)
        b = _enc_date(datetime.date(y, m, d))
        out.append(f'    ("{b.hex()}", {y}, {m}, {d}),')
    out.append("];")

    out.append("pub const TIME_VECTORS: &[(&str, u8, u8, u8, u32)] = &[")
    for _ in range(count):
        h = random.randint(0, 23)
        mi = random.randint(0, 59)
        s = random.randint(0, 59)
        us = random.randint(0, 999999)
        b = _enc_time(datetime.time(h, mi, s, us))
        out.append(f'    ("{b.hex()}", {h}, {mi}, {s}, {us}),')
    out.append("];")

    out.append("pub const DATETIME_VECTORS: &[(&str, i32, u8, u8, u8, u8, u8, u32)] = &[")
    for _ in range(count):
        y = random.randint(1, 9999)
        m = random.randint(1, 12)
        d = random.randint(1, 28)
        h = random.randint(0, 23)
        mi = random.randint(0, 59)
        s = random.randint(0, 59)
        us = random.randint(0, 999999)
        b = _enc_datetime(datetime.datetime(y, m, d, h, mi, s, us))
        out.append(f'    ("{b.hex()}", {y}, {m}, {d}, {h}, {mi}, {s}, {us}),')
    out.append("];")

    out.append("pub const DECIMAL_VECTORS: &[(&str, &str)] = &[")
    for _ in range(count * 2):
        n = random.randint(1, 12)
        raw = bytes(random.randint(0, 255) for _ in range(n))
        val = str(_decode_decimal(raw, type("C", (), {"col_type": 9})()))
        out.append(f'    ("{raw.hex()}", "{val}"),')
    out.append("];")

    print("\n".join(out))


if __name__ == "__main__":
    main()
