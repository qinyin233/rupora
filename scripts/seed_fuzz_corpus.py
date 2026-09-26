"""Build small, synthetic starting corpora for the three libFuzzer targets.

The generated files live under the ignored fuzz/corpus directory. Keep the
source cases here so reviewers can see what a fuzz run starts from.
"""

from pathlib import Path


CORPUS = Path(__file__).resolve().parents[1] / "fuzz" / "corpus"


def number(value: int) -> bytes:
    output = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        output.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(output)


def trace(source: str, edits: list[tuple[int, int, int, str]]) -> bytes:
    encoded = source.encode("utf-8")
    output = bytearray(number(len(encoded)) + encoded)
    for flags, first, second, replacement in edits:
        encoded_replacement = replacement.encode("utf-8")
        output.append(flags)
        output.extend(number(first))
        output.extend(number(second))
        output.extend(number(len(encoded_replacement)))
        output.extend(encoded_replacement)
    return bytes(output)


SEEDS = {
    "markdown_pipeline": {
        "nested-markdown": (
            "# 标题🙂\n\n- [x] **粗体** 与 `代码`\n"
            "- [ ] [链接](https://example.invalid/路径)\n\n"
            "[^注]: 脚注内容\n\n引用[^注] &amp; 文字\n"
        ).encode("utf-8"),
        "line-endings": b"> quote\r\n> continuation\r\n\r\n```rust\r\nlet x = 1;\r\n```\r\n",
    },
    "table_parser": {
        "gfm-unicode": "| 名称 | 值 |\n| :--- | ---: |\n| 中文🙂 | `a|b` |\n".encode("utf-8"),
        "escaped-pipe": b"before\n\n| a\\|b | c |\n| --- | --- |\n| x | y |\n\nafter\n",
    },
    "wysiwyg_projection": {
        "unicode-multi-edit": trace(
            "中文🙂尾", [(0, 1, 3, "新"), (1, 2, 2, "🙂"), (2, 0, 1, "前")]
        ),
        "nested-format-media": trace(
            "**前**![*中*](x)**后**", [(1, 0, 1, "新"), (2, 1, 3, "🙂")]
        ),
        "footnote-inline-code": trace("[^/`x`]", [(0, 4, 4, "Z")]),
        "list-tabs-and-code": trace("x\n*\t\t\t\t\t\t\t#*", [(0, 1, 2, "🙂")]),
        "crlf-task-list": trace(
            "- [ ] 任务\r\n- [x] 完成\r\n", [(0, 2, 2, "新"), (1, 0, 0, "前")]
        ),
        "quote-tab-list-follow-up": trace(
            ">\t- a  b", [(0, 4, 5, ""), (1, 4, 4, "X")]
        ),
        "quote-tab-indented-code": trace(">\t\ta", [(0, 4, 4, "X")]),
        "literal-indented-quote": trace("x\n\t> ", [(0, 4, 4, "新🙂")]),
        "literal-indented-quote-cr": trace("甲🙂\r    >\t", [(0, 7, 7, "新")]),
        "empty-quote-tail-cr": trace("> 甲🙂\r> ", [(0, 7, 7, "尾")]),
        "heading-leading-space-follow-up": trace(
            "## a  b", [(0, 0, 1, ""), (1, 0, 0, "X")]
        ),
        "table-cell-space-follow-up": trace(
            "| a  b | c |\n| - | - |\n| d | e |", [(0, 3, 4, ""), (1, 3, 3, "X")]
        ),
        "table-separator-input": trace(
            "| a | b |\n| - | - |\n| c | d |", [(0, 3, 3, "X"), (1, 2, 2, " ")]
        ),
        "container-marker-input": trace(
            "> - hello\n- [ ] task\n1) item", [(0, 1, 1, "X"), (1, 5, 5, " ")]
        ),
        "multiline-inline-code-input": trace(
            "`中\n🙂`", [(0, 1, 1, "X"), (1, 2, 3, "Y")]
        ),
        "autolink-invalid-edit": trace(
            "a <https://x.test> z", [(0, 2, 3, "新"), (1, 3, 3, "🙂")]
        ),
        "email-autolink-caret": trace("a <user@x.test> z", [(0, 5, 8, "x")]),
        "collapsed-reference-label-edit": trace(
            "a [label][] z\n\n[label]: https://x.test", [(0, 2, 3, "新")]
        ),
        "shortcut-reference-label-edit": trace(
            "a [label] z\n\n[label]: https://x.test", [(0, 2, 3, "新")]
        ),
        "long-unicode-offset": trace("a" * 1024 + "中文🙂尾", [(0, 1024, 1027, "新🙂")]),
    },
}


def main() -> None:
    for target, cases in SEEDS.items():
        directory = CORPUS / target
        directory.mkdir(parents=True, exist_ok=True)
        for name, data in cases.items():
            (directory / name).write_bytes(data)
        print(f"{target}: {len(cases)} synthetic seeds")


if __name__ == "__main__":
    main()
