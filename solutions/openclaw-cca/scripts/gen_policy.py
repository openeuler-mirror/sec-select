#!/usr/bin/env python3
"""
从 RBS 返回的 JWT（header.payload.signature）中提取 CCA/vCCA Realm 度量值，
填充对应模板的 predefined_values，输出 STANDARD base64 编码的策略。

用法：
    gen_policy.py [--type cca|vcca] <jwt>
    gen_policy.py [--type cca|vcca] <file>   # 文件内容为单个 JWT

模板：
    cca  → cca_template.rego   （默认）
    vcca → vcca_template.rego
"""

import sys
import json
import base64
import re
import os

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

TEMPLATE_DIR = os.path.join(SCRIPT_DIR, "policy_template")

PROFILES = {
    "cca": {
        "template": os.path.join(TEMPLATE_DIR, "cca.rego"),
        "jwt_path": ("cca", "realm_token"),
        "keys": ["cca_rpv", "cca_rim", "cca_rem0", "cca_rem1", "cca_rem2", "cca_rem3"],
    },
    "vcca": {
        "template": os.path.join(TEMPLATE_DIR, "vcca.rego"),
        "jwt_path": ("virt_cca", "realm_token"),
        "keys": ["vcca_rpv", "vcca_rim", "vcca_rem0", "vcca_rem1", "vcca_rem2", "vcca_rem3"],
    },
}


def b64url_decode(segment: str) -> bytes:
    segment += "=" * (-len(segment) % 4)
    return base64.urlsafe_b64decode(segment)


def jwt_payload(token: str) -> dict:
    parts = token.strip().split(".")
    if len(parts) < 2:
        raise ValueError(f"不是合法的 JWT（缺少 payload 段）：{token[:40]!r}")
    return json.loads(b64url_decode(parts[1]))


def parse_args(argv):
    args = argv[1:]
    profile_name = "cca"
    if len(args) >= 2 and args[0] == "--type":
        profile_name = args[1]
        args = args[2:]
    if len(args) != 1:
        print(
            f"用法：{argv[0]} [--type cca|vcca] <jwt 或文件路径>",
            file=sys.stderr,
        )
        sys.exit(1)
    if profile_name not in PROFILES:
        print(f"错误：未知类型 {profile_name!r}，支持：{', '.join(PROFILES)}", file=sys.stderr)
        sys.exit(1)
    return profile_name, args[0]


def main():
    profile_name, arg = parse_args(sys.argv)
    profile = PROFILES[profile_name]

    if os.path.isfile(arg):
        with open(arg) as f:
            arg = f.read().strip()

    try:
        payload = jwt_payload(arg)
    except Exception as e:
        print(f"错误：JWT 解析失败：{e}", file=sys.stderr)
        sys.exit(1)

    top_key, sub_key = profile["jwt_path"]
    realm = payload.get(top_key, {}).get(sub_key)
    if not realm:
        print(
            f"错误：JWT payload 中未找到 {top_key}.{sub_key} 字段",
            file=sys.stderr,
        )
        sys.exit(1)

    with open(profile["template"]) as f:
        policy = f.read()

    for key in profile["keys"]:
        value = realm.get(key)
        if value is None:
            print(f"警告：JWT {top_key}.{sub_key} 中缺少字段 {key!r}", file=sys.stderr)
            continue
        policy = re.sub(
            rf'("{re.escape(key)}":\s*")[^"]*(")',
            lambda m, v=value: m.group(1) + v + m.group(2),
            policy,
        )
    print("策略如下(base64编码):")
    print(base64.b64encode(policy.encode()).decode())


if __name__ == "__main__":
    main()
