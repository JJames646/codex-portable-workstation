const SKILL_CATEGORY_LABELS: Record<string, string> = {
  "business-ops": "商业运营",
  "data-analysis": "数据分析",
  "design-media": "设计与媒体",
  "dev-programming": "开发编程",
  automation: "自动化",
  development: "开发工具",
  education: "教育学习",
  marketing: "市场营销",
  office: "办公创作",
  productivity: "效率工具",
  professional: "专业工具",
  research: "研究分析",
  security: "安全防护",
  writing: "写作创作",
};

const HAS_CHINESE = /[\u3400-\u9fff]/u;

/**
 * SkillHub 分类键仍作为筛选参数保留，这里只负责用户界面的中文显示。
 */
export function skillCategoryLabel(category: string): string {
  const normalized = category.trim();
  if (!normalized) return "其他分类";
  if (normalized === "全部" || HAS_CHINESE.test(normalized)) return normalized;
  return SKILL_CATEGORY_LABELS[normalized.toLowerCase()] ?? "其他分类";
}
