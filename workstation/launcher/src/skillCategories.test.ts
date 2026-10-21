import { describe, expect, it } from "vitest";

import { skillCategoryLabel } from "./skillCategories";

describe("skillCategoryLabel", () => {
  it("translates SkillHub category keys for display", () => {
    expect(skillCategoryLabel("data-analysis")).toBe("数据分析");
    expect(skillCategoryLabel("dev-programming")).toBe("开发编程");
    expect(skillCategoryLabel("business-ops")).toBe("商业运营");
    expect(skillCategoryLabel("design-media")).toBe("设计与媒体");
  });

  it("keeps an existing Chinese category unchanged", () => {
    expect(skillCategoryLabel("图像处理")).toBe("图像处理");
    expect(skillCategoryLabel("全部")).toBe("全部");
  });

  it("does not expose an unknown English key", () => {
    expect(skillCategoryLabel("future-category")).toBe("其他分类");
  });
});
