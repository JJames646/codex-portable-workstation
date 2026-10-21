import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MarkdownTutorial } from "./MarkdownTutorial";

describe("MarkdownTutorial", () => {
  it("renders GFM content as readable document markup", () => {
    const html = renderToStaticMarkup(
      <MarkdownTutorial
        engine="openclaw"
        markdown={`# 项目介绍

**重点内容**

> 💡 阅读提示

| 项目 | 说明 |
| --- | --- |
| Gateway | 本地网关 |

- 第一项
- 第二项

\`inline code\`

~~~bash
openclaw doctor
~~~
`}
      />,
    );

    expect(html).toContain("<h1>项目介绍</h1>");
    expect(html).toContain("<strong>重点内容</strong>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain('class="tutorial-table-scroll"');
    expect(html).toContain("<table>");
    expect(html).toContain("<pre>");
  });

  it("does not execute raw HTML from a tutorial", () => {
    const html = renderToStaticMarkup(
      <MarkdownTutorial
        engine="codex"
        markdown={'正常文字\n\n<script>alert("x")</script>'}
      />,
    );

    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });
});
