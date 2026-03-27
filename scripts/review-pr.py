#!/usr/bin/env python3
"""
PR Code Review Script using Claude API
"""

import sys
import os
import anthropic


def review_code(diff_content: str) -> str:
    """使用 Claude 审查代码变更"""

    api_key = os.getenv("ANTHROPIC_API_KEY")
    base_url = os.getenv("ANTHROPIC_BASE_URL")

    if not api_key:
        return "❌ 错误：未设置 ANTHROPIC_API_KEY"

    # 初始化客户端
    client_kwargs = {"api_key": api_key}
    if base_url:
        client_kwargs["base_url"] = base_url

    client = anthropic.Anthropic(**client_kwargs)

    # 限制 diff 大小
    if len(diff_content) > 50000:
        return "⚠️ Diff 过大（>50KB），跳过审查"

    try:
        message = client.messages.create(
            model="claude-opus-4-6",
            max_tokens=2048,
            messages=[
                {
                    "role": "user",
                    "content": f"""请审查以下代码变更，指出潜在问题、改进建议和安全隐患。

**审查重点：**
- 代码质量和可维护性
- 性能问题
- 安全隐患
- 与项目架构的一致性
- 测试覆盖

**格式要求：**
- 按严重程度分类（🔴 严重 / 🟡 警告 / 💡 建议）
- 指出具体文件和行号
- 提供修复建议

```diff
{diff_content}
```"""
                }
            ]
        )

        review_text = message.content[0].text
        return f"## 🤖 Claude 代码审查\n\n{review_text}"

    except anthropic.APIError as e:
        return f"❌ API 错误：{str(e)}"
    except Exception as e:
        return f"❌ 审查失败：{str(e)}"


def main():
    if len(sys.argv) < 2:
        print("❌ 用法：python review-pr.py <diff_file>")
        sys.exit(1)

    diff_file = sys.argv[1]

    if not os.path.exists(diff_file):
        print(f"❌ 文件不存在：{diff_file}")
        sys.exit(1)

    with open(diff_file, 'r', encoding='utf-8') as f:
        diff_content = f.read()

    if not diff_content.strip():
        print("⚠️ Diff 为空，无需审查")
        sys.exit(0)

    review = review_code(diff_content)
    print(review)


if __name__ == "__main__":
    main()
