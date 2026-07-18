import type { Category } from "../api/backend";

const categoryOrder: readonly Category[] = [
  "instruction_following",
  "logic",
  "code_review",
  "cli_coding",
];

const categoryLabels: Record<Category, string> = {
  instruction_following: "指令遵循",
  logic: "逻辑推理",
  code_review: "代码审查",
  cli_coding: "CLI 编码",
};

export function CategoryBars({
  scores,
}: {
  scores: Partial<Record<Category, number>>;
}) {
  const presentScores = categoryOrder.flatMap((category) => {
    const score = scores[category];
    return typeof score === "number" && Number.isFinite(score)
      ? [{ category, score }]
      : [];
  });

  return (
    <ul aria-label="各能力分类得分" className="category-bars">
      {presentScores.map(({ category, score }) => {
        const label = categoryLabels[category];
        const decorativeWidth = Math.max(0, Math.min(100, score));
        return (
          <li className="category-row" key={category}>
            <span data-testid="category-label">{label}</span>
            <span aria-hidden="true" className="bar-track">
              <span style={{ width: `${decorativeWidth}%` }} />
            </span>
            <strong aria-label={`${label} ${score.toFixed(1)} 分`}>
              {score.toFixed(1)} 分
            </strong>
          </li>
        );
      })}
    </ul>
  );
}
