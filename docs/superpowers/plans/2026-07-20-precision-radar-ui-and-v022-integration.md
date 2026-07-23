# Precision Radar UI and v0.2.2 Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the application’s visual hierarchy around the approved “precision radar” direction, remove the two reported layout defects, and seal the verified result as a v0.2.2 candidate.

**Architecture:** Establish one offline token system and constrained application shell first, then refactor Home and Manual setup markup around stable semantic regions. Migrate remaining page styles to the same tokens, perform responsive and accessibility QA, and only then update the repository’s sealed version contracts and packaging metadata.

**Tech Stack:** React 19, TypeScript 5.8, CSS custom properties, Vitest, Testing Library, axe-core, Tauri 2, Rust, Windows packaging scripts.

## Global Constraints

- Run this plan only after the trusted CLI and client-identification plans pass.
- Visual direction is “precision radar / laboratory instrument panel.”
- Use warm off-white, deep ink green, radar teal, and limited amber accents.
- Do not add external fonts, remote images, tracking, or network dependencies.
- Do not use CSS gradients; use solid surfaces, borders, pseudo-elements, and a local data-URI SVG grid.
- Avoid excessive pills, oversized titles, decorative cards, and large empty hero regions.
- Preserve semantic headings, visible labels, keyboard order, `aria-live`, focus rings, forced-colors support, and reduced-motion support.
- Light and dark themes must retain WCAG AA text contrast and 3:1 non-text contrast.
- Check at `1920×1080`, `1366×768`, `1024px` width, and a narrow single-column window.
- No horizontal overflow, overlap, clipped theme control, or layout jump is acceptable.
- Public downloads remain inactive until clean Windows 10/11 x64 release validation.
- Target candidate version is exactly `0.2.2`; no current-public-release claim is allowed.

---

## File Structure

- `apps/desktop/src/styles/tokens.css` — complete light/dark visual tokens and local grid pattern.
- `apps/desktop/src/styles/app.css` — shell, Home, shared controls, state cards, responsive rules.
- `apps/desktop/src/components/AppShell.tsx` — constrained topbar inner layout.
- `apps/desktop/src/pages/HomePage.tsx` — compact hero, section headers, guidance strip, target card hierarchy.
- `apps/desktop/src/pages/HomePage.test.tsx` — semantic Home structure and actions.
- `apps/desktop/src/pages/ManualRunPage.tsx` — compact setup hero and one coherent setup panel.
- `apps/desktop/src/pages/ManualRunPage.css` — token-only setup/task layout.
- `apps/desktop/src/components/ClientSelectionPanel.tsx` — classes consumed by the setup visual layout.
- `apps/desktop/src/pages/CliRunPage.css` — token migration for CLI states.
- `apps/desktop/src/pages/ResultsHistory.css` — token migration for result/history states.
- `apps/desktop/src/test/accessibility.test.tsx` — token contrast, no-gradient, structure, axe, and forced-colors contracts.
- Version/release contract files listed in Task 6.

### Visual Interfaces

Shared layout classes produced by this plan:

```text
topbar-inner
home-hero
hero-data-strip
section-heading-copy
section-pack-meta
section-action
cli-guidance
target-identity
target-source
manual-setup-hero
manual-setup-panel
selection-panel
selection-status
selection-candidates
manual-actions
```

---

### Task 1: Add Failing Visual-System Contracts

**Files:**

- Modify: `apps/desktop/src/test/accessibility.test.tsx`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.test.tsx`

**Interfaces:**

- Consumes: existing accessible page tests.
- Produces: structural and CSS constraints that guide Tasks 2–5.

- [ ] **Step 1: Add the no-gradient and token-only contract**

In `accessibility.test.tsx`, add:

```ts
test("the precision-radar surface uses no CSS gradients", () => {
  const styleFiles = [
    join(sourceRoot, "styles", "app.css"),
    join(sourceRoot, "pages", "ManualRunPage.css"),
    join(sourceRoot, "pages", "CliRunPage.css"),
    join(sourceRoot, "pages", "ResultsHistory.css"),
  ];
  for (const file of styleFiles) {
    const source = readFileSync(file, "utf8");
    expect(source, file).not.toMatch(
      /(?:linear|radial|conic|repeating-radial)-gradient\s*\(/i,
    );
  }
});
```

Add:

```ts
function withoutCssUrlPayloads(source: string): string {
  const urlStart = /\burl\s*\(/gi;
  let cursor = 0;
  let output = "";
  let match: RegExpExecArray | null;

  while ((match = urlStart.exec(source))) {
    output += `${source.slice(cursor, match.index)}url()`;
    let index = urlStart.lastIndex;
    let depth = 1;
    let quote: '"' | "'" | null = null;

    while (index < source.length && depth > 0) {
      const character = source[index];
      if (character === "\\") {
        index += 2;
        continue;
      }
      if (quote) {
        if (character === quote) quote = null;
      } else if (character === '"' || character === "'") {
        quote = character;
      } else if (character === "(") {
        depth += 1;
      } else if (character === ")") {
        depth -= 1;
      }
      index += 1;
    }

    cursor = index;
    urlStart.lastIndex = index;
  }

  return output + source.slice(cursor);
}

test("page styles consume shared color tokens instead of hard-coded colors", () => {
  const dataUrlFixture =
    ".fixture { background: url(\"data:image/svg+xml,<svg fill='rgb(1 2 3)' stroke='#fff'/>\"); }";
  expect(withoutCssUrlPayloads(dataUrlFixture)).toBe(
    ".fixture { background: url(); }",
  );

  for (const file of [
    "ManualRunPage.css",
    "CliRunPage.css",
    "ResultsHistory.css",
  ]) {
    const source = readFileSync(join(sourceRoot, "pages", file), "utf8");
    const declarations = withoutCssUrlPayloads(source);
    expect(declarations, file).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(declarations, file).not.toMatch(
      /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/i,
    );
    expect(source, file).toMatch(/var\(--text-primary\)/);
    expect(source, file).toMatch(/var\(--border\)/);
  }
});
```

- [ ] **Step 2: Add Home structure assertions**

After rendering Home, assert:

```ts
const heading = screen.getByRole("heading", { name: "选择要体检的 AI" });
const hero = screen.getByTestId("home-hero");
expect(hero).toHaveClass("home-hero");
expect(hero).toContainElement(heading);
const cliSection = screen.getByRole("region", { name: "编程 CLI" });
expect(
  within(cliSection).getByRole("button", { name: "重新检测 CLI" }),
).toBeInTheDocument();
expect(
  within(cliSection).getByText("CLI 快速体检 · v1.0.0"),
).toBeInTheDocument();

const guidance = screen.getByTestId("cli-guidance");
expect(guidance).toHaveClass("cli-guidance");
expect(guidance).toHaveTextContent("新增 PATH 目录后请重启应用");
expect(
  guidance.closest(".section-heading-actions, .section-action"),
).toBeNull();
expect(guidance.parentElement).toBe(cliSection);

const sectionHeader = cliSection.querySelector(":scope > .section-heading");
const targetGrid = cliSection.querySelector(":scope > .target-grid");
const childOrder = Array.from(cliSection.children);
expect(childOrder.indexOf(sectionHeader!))
  .toBeLessThan(childOrder.indexOf(guidance));
expect(childOrder.indexOf(guidance))
  .toBeLessThan(childOrder.indexOf(targetGrid!));
```

Add a deferred refresh test: after clicking “重新检测 CLI”, the existing
hero and target cards remain mounted, the button is disabled with text
“正在重新检测…”, and an inline `role="status"` is present. Locate that status
by role inside `.cli-refresh-control` without an accessible-name filter, then
assert its text is `正在重新检测 CLI`; this preserves the approved
`<span className="sr-only" role="status">…</span>` shape. Resolve the deferred
bootstrap and assert the same card DOM region updates without a full-page
loading replacement. Reject a second deferred bootstrap and assert the old
cards remain with an inline retryable error. Use soft presence assertions so
both controlled promises are explicitly settled on the RED path.

- [ ] **Step 3: Add Manual setup structure assertions**

In `ManualRunPage.test.tsx`, assert setup mode contains:

```ts
const heading = screen.getByRole("heading", {
  name: "ChatGPT 客户端快速体检",
});
const setupRoot = screen.getByRole("main");
const hero = screen.getByTestId("manual-setup-hero");
const panel = screen.getByTestId("manual-setup-panel");
expect(hero).toHaveClass("manual-setup-hero");
expect(panel).toHaveClass("manual-setup-panel");
expect(hero).toContainElement(heading);
expect(hero.parentElement).toBe(setupRoot);
expect(panel.parentElement).toBe(setupRoot);
expect(hero).not.toContainElement(panel);
expect(panel).not.toContainElement(hero);
expect(Array.from(setupRoot.children).indexOf(hero))
  .toBeLessThan(Array.from(setupRoot.children).indexOf(panel));
expect(screen.getByLabelText("当前显示的模型"))
  .toHaveAttribute("autocomplete", "off");
expect(screen.getByRole("button", { name: "开始快速体检" }))
  .toBeInTheDocument();
```

- [ ] **Step 4: Run the targeted tests and verify expected failures**

Run:

```powershell
npm test --workspace apps/desktop -- src/test/accessibility.test.tsx src/pages/HomePage.test.tsx src/pages/ManualRunPage.test.tsx
```

Expected: no-gradient, token-only, and new structure assertions fail against
the current UI.

- [ ] **Step 5: Commit only the failing contracts**

```powershell
git add apps/desktop/src/test/accessibility.test.tsx apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/pages/ManualRunPage.test.tsx
git commit -m "test: define precision radar UI contracts"
```

---

### Task 2: Rebuild Tokens and the Application Shell

**Files:**

- Modify: `apps/desktop/src/styles/tokens.css`
- Modify: `apps/desktop/src/styles/app.css`
- Modify: `apps/desktop/src/components/AppShell.tsx`
- Test: `apps/desktop/src/test/accessibility.test.tsx`
- Test: `apps/desktop/src/app/App.test.tsx`

**Interfaces:**

- Consumes: Task 1 visual contracts.
- Produces: all shared colors, spacing, type, shadows, local grid, and
  `topbar-inner`.

- [ ] **Step 1: Replace the token palette**

Use these exact light tokens:

```css
:root {
  color-scheme: light;
  --canvas: #f2f3ee;
  --panel: #fbfcf8;
  --panel-raised: #ffffff;
  --surface-muted: #e7eeea;
  --surface-strong: #d6e3de;
  --text-primary: #102824;
  --text-muted: #4c6660;
  --border: #b7c9c3;
  --border-strong: #607d76;
  --brand: #0b7067;
  --brand-strong: #07584f;
  --brand-soft: #d7ebe5;
  --mineral: #315e70;
  --success: #176c4f;
  --success-soft: #dceee4;
  --warning: #865400;
  --warning-soft: #f5e6bf;
  --danger: #96363b;
  --danger-soft: #f5dfe1;
  --focus: #145da0;
  --on-brand: #ffffff;
  --grid-pattern: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='40' viewBox='0 0 40 40'%3E%3Cpath d='M40 0H0V40' fill='none' stroke='%230b7067' stroke-opacity='.055'/%3E%3C/svg%3E");
  --shadow-sm: 0 1px 2px rgb(16 40 36 / 8%);
  --shadow-md: 0 14px 34px rgb(16 40 36 / 10%);
  --radius-sm: 0.4rem;
  --radius-md: 0.75rem;
  --space-1: 0.25rem;
  --space-2: 0.5rem;
  --space-3: 0.75rem;
  --space-4: 1rem;
  --space-5: 1.5rem;
  --space-6: 2rem;
  --space-7: 3rem;
  --font-display: "Segoe UI Variable Display", "Microsoft YaHei UI",
    "Segoe UI", sans-serif;
  --font-body: "Segoe UI Variable Text", "Microsoft YaHei UI",
    "Segoe UI", sans-serif;
}
```

Use these dark tokens in both explicit and system-dark blocks:

```css
--canvas: #0a1513;
--panel: #10221f;
--panel-raised: #152b27;
--surface-muted: #1c3732;
--surface-strong: #284a43;
--text-primary: #edf6f2;
--text-muted: #b6cbc5;
--border: #3a5a53;
--border-strong: #78978f;
--brand: #78d4c5;
--brand-strong: #a0e7dc;
--brand-soft: #173e37;
--mineral: #91bfd0;
--success: #88d8af;
--success-soft: #173c2f;
--warning: #efc36d;
--warning-soft: #45371c;
--danger: #ff9ea4;
--danger-soft: #47272b;
--focus: #9dcaff;
--on-brand: #071a17;
--grid-pattern: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='40' viewBox='0 0 40 40'%3E%3Cpath d='M40 0H0V40' fill='none' stroke='%2378d4c5' stroke-opacity='.055'/%3E%3C/svg%3E");
```

- [ ] **Step 2: Replace gradients and oversized shared typography**

In `app.css`:

```css
body {
  background-color: var(--canvas);
  background-image: var(--grid-pattern);
  background-size: 2.5rem 2.5rem;
}

.app-shell h1 {
  max-width: 22ch;
  font-size: clamp(2rem, 4.4vw, 3.75rem);
  line-height: 1.04;
  letter-spacing: -0.035em;
}
```

Replace the hero gradient rule with:

```css
.hero::after,
.evidence-hero::after {
  content: "";
  width: 4rem;
  height: 0.2rem;
  margin-top: 0.35rem;
  border-left: 0.65rem solid var(--warning);
  background: var(--brand);
}
```

Replace radar gradients with borders and box shadows:

```css
.brand-mark {
  background: var(--panel-raised);
}

.brand-mark::before {
  inset: 0.4rem;
  border: 1px solid var(--border-strong);
}

.brand-mark::after {
  width: 0.38rem;
  height: 0.38rem;
  background: var(--warning);
  box-shadow: 0 0 0 0.28rem var(--panel-raised);
}

.target-card::after {
  width: 0.36rem;
  height: 0.36rem;
  border: 0;
  background: var(--brand);
  box-shadow:
    0 0 0 0.42rem var(--panel),
    0 0 0 0.5rem var(--border),
    0 0 0 0.9rem var(--panel),
    0 0 0 0.98rem var(--border);
}
```

- [ ] **Step 3: Constrain the topbar with an inner wrapper**

Change `AppShell.tsx`:

```tsx
<header className="topbar">
  <div className="topbar-inner">
    <Link className="brand" to="/">
      <span aria-hidden="true" className="brand-mark" />
      <span>{t("app.name")}</span>
    </Link>
    <nav aria-label={t("nav.label")} className="main-navigation">
      <Link
        aria-current={startActive ? "page" : undefined}
        className={navClassName(startActive)}
        to="/"
      >
        {t("nav.start")}
      </Link>
      <Link
        aria-current={historyActive ? "page" : undefined}
        className={navClassName(historyActive)}
        to="/history"
      >
        {t("nav.history")}
      </Link>
    </nav>
    <ThemeToggle />
  </div>
</header>
```

Use:

```css
.topbar {
  min-height: 3.75rem;
  padding: 0;
}

.topbar-inner {
  display: grid;
  width: min(76rem, calc(100% - clamp(1.25rem, 4vw, 3rem)));
  min-height: 3.75rem;
  margin: 0 auto;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: clamp(0.75rem, 2vw, 1.5rem);
}
```

Move the existing `48rem` responsive topbar rules from `.topbar` to
`.topbar-inner`.

- [ ] **Step 4: Run shell, contrast, and no-gradient tests**

Run:

```powershell
npm test --workspace apps/desktop -- src/app/App.test.tsx src/test/accessibility.test.tsx
```

Expected: shell structure, theme restoration, token contrast, forced-colors,
reduced-motion, and no-gradient tests pass for shared styles; page token-only
tests still fail until Tasks 4–5.

- [ ] **Step 5: Commit tokens and shell**

```powershell
git add apps/desktop/src/styles/tokens.css apps/desktop/src/styles/app.css apps/desktop/src/components/AppShell.tsx apps/desktop/src/test/accessibility.test.tsx apps/desktop/src/app/App.test.tsx
git commit -m "style: establish precision radar shell"
```

---

### Task 3: Recompose the Home Page

**Files:**

- Modify: `apps/desktop/src/pages/HomePage.tsx`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/styles/app.css`
- Test: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**

- Consumes: CLI status/source from the first plan and shared tokens from Task
  2.
- Produces: compact Home hero, aligned section header, separate CLI guidance,
  stable in-place CLI refresh, and balanced provider cards.

- [ ] **Step 1: Refactor `TargetGroup` props and markup**

Use:

```tsx
function TargetGroup({
  title,
  description,
  targets,
  pack,
  id,
  action,
  guidance,
}: {
  title: string;
  description: string;
  targets: TargetAvailability[];
  pack: PackSummary;
  id: string;
  action?: ReactNode;
  guidance?: ReactNode;
}) {
  const titleId = `${id}-title`;
  return (
    <section aria-labelledby={titleId} className="target-section">
      <header className="section-heading">
        <div className="section-heading-copy">
          <p className="section-kicker">{description}</p>
          <h2 id={titleId}>{title}</h2>
          <div className="section-pack-meta">
            <span>{pack.title} · v{pack.version}</span>
            <span>{pack.taskCount} 道任务 · 预计 {pack.estimatedMinutes} 分钟</span>
          </div>
        </div>
        {action ? <div className="section-action">{action}</div> : null}
      </header>
      {guidance}
      <div className="target-grid">
        {targets.map((target) => (
          <TargetCard key={target.kind} pack={pack} target={target} />
        ))}
      </div>
    </section>
  );
}
```

Remove `.section-heading-actions`, `.pack-summary`, and the long text from
the action slot.

- [ ] **Step 2: Compact the Home hero**

Use:

```tsx
<section
  aria-labelledby="home-title"
  className="hero home-hero"
  data-testid="home-hero"
>
  <p className="eyebrow">本地优先 · 结果按目标分别记录</p>
  <h1 id="home-title">选择要体检的 AI</h1>
  <p className="hero-summary">
    客户端逐题复制粘贴，CLI 在专用临时目录自动执行。
  </p>
  <div className="hero-data-strip" aria-label="体检边界">
    <span>原始数据仅存本机</span>
    <span>使用你自己的订阅额度</span>
    <span>衡量端到端产品表现</span>
  </div>
</section>
```

Keep the complete cost/privacy notice lower on the page.

- [ ] **Step 3: Separate CLI guidance and keep refresh in place**

Pass:

```tsx
action={
  <div className="cli-refresh-control">
    <button
      className="secondary-action"
      disabled={refreshing}
      onClick={refreshBootstrap}
      type="button"
    >
      {refreshing ? "正在重新检测…" : "重新检测 CLI"}
    </button>
    {refreshing ? (
      <span className="sr-only" role="status">
        正在重新检测 CLI
      </span>
    ) : null}
    {refreshError ? (
      <p className="inline-error" role="alert">
        {refreshError}
      </p>
    ) : null}
  </div>
}
guidance={
  <p className="cli-guidance" data-testid="cli-guidance">
    已继承 PATH 目录内的变化可以立即刷新；安装程序新增 PATH
    目录后请重启应用，再重新检测。
  </p>
}
```

Refactor bootstrap state so initial loading still uses the dedicated loading
page, but `refreshBootstrap` retains the last successful `Bootstrap` while
it requests a replacement. On refresh failure, keep that data and expose
only the sanitized local error beside the refresh control. Remove the
`attempt`-driven effect that resets the whole page to `loading`.

In `TargetCard`, render a redundant non-color indicator:

```tsx
<p
  aria-label={statusLabel}
  className={
    reason
      ? "target-status status-warning"
      : "target-status status-ready"
  }
  role="status"
>
  <span aria-hidden="true" className="status-indicator" />
  <span>{status}</span>
</p>
```

- [ ] **Step 4: Apply balanced Home CSS**

Use:

```css
.home-page {
  padding-top: clamp(1.75rem, 4vw, 3rem);
}

.home-hero {
  max-width: 62rem;
  margin-bottom: var(--space-7);
}

.hero-data-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem 1.25rem;
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 0.88rem;
}

.hero-data-strip span::before {
  content: "";
  display: inline-block;
  width: 0.42rem;
  height: 0.42rem;
  margin-right: 0.45rem;
  border-radius: 50%;
  background: var(--brand);
  vertical-align: 0.08rem;
}

.section-heading {
  align-items: center;
}

.section-heading-copy {
  display: grid;
  gap: var(--space-1);
}

.section-pack-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 1rem;
  color: var(--text-muted);
  font-size: 0.88rem;
  font-variant-numeric: tabular-nums;
}

.cli-guidance {
  margin: 0 0 var(--space-4);
  padding: 0.65rem 0.8rem;
  border-left: 0.2rem solid var(--mineral);
  background: var(--surface-muted);
  color: var(--text-muted);
  font-size: 0.86rem;
}

.target-grid {
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-4);
}

.target-card {
  min-height: 14.5rem;
  padding: var(--space-5);
  border-top-width: 0.2rem;
  box-shadow: var(--shadow-sm);
}

.target-card:hover {
  border-color: var(--border-strong);
  box-shadow: var(--shadow-md);
}

.app-shell .target-card .target-status.status-ready {
  color: var(--success);
}

.app-shell .target-card .target-status.status-warning {
  color: var(--warning);
}

.status-indicator {
  width: 0.55rem;
  height: 0.55rem;
  border: 0.12rem solid currentColor;
  border-radius: 50%;
  box-shadow: 0 0 0 0.18rem var(--panel);
}
```

Delete the old `.target-status::before { content: "状态"; ... }` pill rule;
the dot, visible status text, and `aria-label` now carry the redundant state.

At `48rem`, make the grid one column and left-align section action. Keep
button and text within their section.

- [ ] **Step 5: Run Home and accessibility tests**

Run:

```powershell
npm test --workspace apps/desktop -- src/pages/HomePage.test.tsx src/test/accessibility.test.tsx
```

Expected: Home structure, four targets, state copy, cost/privacy copy,
navigation, axe, and no-gradient global tests pass.

- [ ] **Step 6: Commit the Home redesign**

```powershell
git add apps/desktop/src/pages/HomePage.tsx apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/styles/app.css apps/desktop/src/test/accessibility.test.tsx
git commit -m "style: recompose the Home dashboard"
```

---

### Task 4: Recompose the Manual Setup Page

**Files:**

- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.css`
- Modify: `apps/desktop/src/components/ClientSelectionPanel.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.test.tsx`
- Test: `apps/desktop/src/components/ClientSelectionPanel.test.tsx`
- Test: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**

- Consumes: client-identification panel from the second plan.
- Produces: compact hero plus one stable setup panel with balanced fields.

- [ ] **Step 1: Split setup hero from the form panel**

Add a required `selectionPanel: ReactNode` prop to `SetupPage`; the wizard
passes the already-configured `ClientSelectionPanel` from the second plan.
Replace the setup page wrapper with this complete structure:

```tsx
<main
  className="page run-page manual-setup-page"
  id="page-content"
  tabIndex={-1}
>
  <header
    className="manual-setup-hero"
    data-testid="manual-setup-hero"
  >
    <p className="eyebrow">客户端 · 快速体检 · 约 10–15 分钟</p>
    <h1 id="manual-setup-title">{label}快速体检</h1>
    <p className="hero-summary">
      每道题使用一个新空白对话，再把完整回答原样粘贴回来。
    </p>
  </header>

  <section
    aria-labelledby="manual-setup-title"
    className="manual-setup-panel"
    data-testid="manual-setup-panel"
  >
    <aside aria-labelledby="manual-boundary-title" className="manual-boundary">
      <div>
        <p className="section-kicker">费用与隐私</p>
        <h2 id="manual-boundary-title">开始前请确认</h2>
      </div>
      <ul>
        <li>客户端使用可能消耗你自己的订阅额度。</li>
        <li>登录凭据不会交给本工具，原始回答仅保存在本机。</li>
        <li>这里衡量端到端客户端表现，不是底层模型“智商”。</li>
      </ul>
    </aside>

    {selectionPanel}

    <div className="manual-fields">
      <div className="field-stack">
        <label className="field" htmlFor="manual-model">
          <span>当前显示的模型</span>
          <input
            id="manual-model"
            aria-describedby={showModelError ? "model-error" : undefined}
            aria-invalid={showModelError ? "true" : undefined}
            autoComplete="off"
            onChange={(event) => onModelChange(event.target.value)}
            placeholder="例如 GPT-5、Claude Sonnet"
            value={model}
          />
        </label>
        {showModelError ? (
          <p className="form-error" id="model-error" role="alert">
            {showModelError}
          </p>
        ) : null}
      </div>
      <ReasoningEffortField
        emptyLabel="未显示 / 不适用"
        id="manual-reasoning"
        kind={kind}
        label="推理档位（没有显示可留空）"
        onChange={onReasoningEffortChange}
        onValidationChange={onReasoningValidationChange}
        value={reasoningEffort}
      />
    </div>

    <label className="check-row">
      <input
        checked={freshChat}
        onChange={(event) => onFreshChatChange(event.target.checked)}
        type="checkbox"
      />
      <span>我会为每道题新建空白对话</span>
    </label>

    <p className="hint">
      除非题目明确允许，否则关闭联网搜索、工具和连接器；不要追加解释性提示，
      也不要把评分结果发回给 AI。
    </p>
    {error ? (
      <p className="form-error" role="alert">
        {error}
      </p>
    ) : null}
    {busy ? (
      <p aria-live="polite" role="status">
        正在创建本地体检…
      </p>
    ) : null}
    <div className="manual-actions">
      <button
        disabled={
          busy ||
          !freshChat ||
          Boolean(modelError) ||
          Boolean(reasoningError)
        }
        onClick={onStart}
        type="button"
      >
        开始快速体检
      </button>
    </div>
  </section>
</main>
```

The actual `selectionPanel` value is the existing `ClientSelectionPanel`;
it appears after the boundary note and before the fields. Do not render it
on resume review, where the stored target is authoritative.

- [ ] **Step 2: Compress the boundary note**

Use:

```tsx
<aside aria-labelledby="manual-boundary-title" className="manual-boundary">
  <div>
    <p className="section-kicker">费用与隐私</p>
    <h2 id="manual-boundary-title">开始前请确认</h2>
  </div>
  <ul>
    <li>客户端使用可能消耗你自己的订阅额度。</li>
    <li>登录凭据不会交给本工具，原始回答仅保存在本机。</li>
    <li>这里衡量端到端客户端表现，不是底层模型“智商”。</li>
  </ul>
</aside>
```

- [ ] **Step 3: Keep model errors in the correct grid cell**

Wrap the model label and error:

```tsx
<div className="field-stack">
  <label className="field" htmlFor="manual-model">
    <span>当前显示的模型</span>
    <input
      id="manual-model"
      aria-describedby={showModelError ? "model-error" : undefined}
      aria-invalid={showModelError ? "true" : undefined}
      autoComplete="off"
      onChange={(event) => onModelChange(event.target.value)}
      placeholder="例如 GPT-5、Claude Sonnet"
      value={model}
    />
  </label>
  {showModelError ? (
    <p className="form-error" id="model-error" role="alert">
      {showModelError}
    </p>
  ) : null}
</div>
```

Place `ReasoningEffortField` as the second equal-width grid child. Keep the
fresh-chat checkbox and actions outside the two-column field grid.

- [ ] **Step 4: Replace Manual CSS with token-based layout**

Use these setup rules:

```css
.run-page {
  width: min(58rem, calc(100% - clamp(1.5rem, 5vw, 4rem)));
  padding-top: clamp(1.75rem, 4vw, 3rem);
  color: var(--text-primary);
}

.manual-setup-page {
  display: grid;
  gap: var(--space-5);
}

.manual-setup-hero {
  display: grid;
  max-width: 48rem;
  gap: var(--space-2);
}

.manual-setup-hero > * {
  margin-block: 0;
}

.manual-setup-panel {
  display: grid;
  padding: clamp(1rem, 3vw, 1.5rem);
  border: 1px solid var(--border);
  border-top: 0.2rem solid var(--brand);
  border-radius: var(--radius-md);
  gap: var(--space-5);
  background: var(--panel);
  box-shadow: var(--shadow-md);
}

.manual-boundary {
  display: grid;
  grid-template-columns: minmax(10rem, 0.42fr) minmax(0, 1fr);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-left: 0.25rem solid var(--warning);
  gap: var(--space-4);
  background: var(--warning-soft);
}

.manual-boundary h2,
.manual-boundary p,
.manual-boundary ul {
  margin-block: 0;
}

.manual-fields {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-4);
}

.field-stack,
.field,
.reasoning-effort-field {
  display: grid;
  min-width: 0;
  align-content: start;
  gap: var(--space-2);
}

.manual-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
}
```

Use tokens for every existing manual task, warning, error, button, prompt,
and transition-page rule. Remove the conflicting
`.app-shell .run-page { width: min(68rem, ...) }` rule from `app.css`; retain
separate `68rem` widths for CLI and evidence pages only.

At `42rem`, make `.manual-fields` and `.manual-boundary` one column.

- [ ] **Step 5: Style the identification panel without pill overload**

Use:

```css
.selection-panel {
  display: grid;
  padding: var(--space-4);
  border: 1px solid var(--border);
  gap: var(--space-3);
  background: var(--surface-muted);
}

.selection-panel-header {
  display: flex;
  align-items: start;
  justify-content: space-between;
  gap: var(--space-3);
}

.selection-status {
  margin: 0;
  color: var(--text-muted);
}

.selection-candidates {
  display: grid;
  margin: 0;
  padding: 0;
  gap: var(--space-2);
  list-style: none;
}
```

Only source/confidence state may use one compact status marker; model name,
instructions, and buttons remain normal text.

- [ ] **Step 6: Run manual, panel, and accessibility tests**

Run:

```powershell
npm test --workspace apps/desktop -- src/pages/ManualRunPage.test.tsx src/components/ClientSelectionPanel.test.tsx src/test/accessibility.test.tsx
```

Expected: setup structure, auto-detection fallback, form validation, task
workflow, axe, token-only Manual CSS, and no-gradient tests pass.

- [ ] **Step 7: Commit Manual redesign**

```powershell
git add apps/desktop/src/pages/ManualRunPage.tsx apps/desktop/src/pages/ManualRunPage.css apps/desktop/src/components/ClientSelectionPanel.tsx apps/desktop/src/pages/ManualRunPage.test.tsx apps/desktop/src/components/ClientSelectionPanel.test.tsx apps/desktop/src/styles/app.css apps/desktop/src/test/accessibility.test.tsx
git commit -m "style: rebuild the manual setup flow"
```

---

### Task 5: Unify CLI, Results, Responsive, and Visual QA

**Files:**

- Modify: `apps/desktop/src/pages/CliRunPage.css`
- Modify: `apps/desktop/src/pages/ResultsHistory.css`
- Modify: `apps/desktop/src/styles/app.css`
- Modify: `apps/desktop/src/test/accessibility.test.tsx`
- Test: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Test: `apps/desktop/src/pages/HistoryPage.ui.test.tsx`
- Test: `apps/desktop/src/pages/ResultPage.test.tsx`

**Interfaces:**

- Consumes: shared tokens and page components.
- Produces: token-only remaining pages and responsive visual evidence.

- [ ] **Step 1: Apply the exact color migration map**

In `CliRunPage.css` and `ResultsHistory.css`, replace hard-coded colors using:

| Existing role | Replacement |
| --- | --- |
| `#102a25`, `#17312c`, `#29463f` | `var(--text-primary)` |
| `#35514b`, `#43655e`, `#557069`, `#6a7f79` | `var(--text-muted)` |
| `#fbfdfc`, `#f8fbfa`, `#f7faf9` | `var(--panel)` |
| `#fff`, `#ffffff` surfaces | `var(--panel-raised)` |
| danger-button foreground | `var(--on-brand)` |
| `#edf6f3`, `#eef6f3`, `#e9f3f0`, `#e4f1ed` | `var(--brand-soft)` |
| `#d1dfdb`, `#c8d9d4`, `#d5e1de`, `#dce7e4` | `var(--border)` |
| `#8eaaa3`, `#9fc2b9`, `#a9c0ba`, `#aebdb9` | `var(--border-strong)` |
| `#147d70`, `#116d62`, `#0f675d`, `#0c594f` | `var(--brand)` / `var(--brand-strong)` by state |
| pale amber surfaces | `var(--warning-soft)` |
| amber text/borders | `var(--warning)` |
| pale red surfaces | `var(--danger-soft)` |
| red text/borders/buttons | `var(--danger)` |
| disabled surfaces/text | `var(--surface-muted)` / `var(--text-muted)` |

Remove every gradient layer. Use the corresponding solid semantic surface;
score bars use `background: var(--brand)`.

- [ ] **Step 2: Normalize panel geometry**

Across CLI and result/history styles:

```css
border-radius: var(--radius-md);
box-shadow: var(--shadow-sm);
```

Use `var(--space-*)` gaps and padding. Primary score cards may use
`border-top: 0.2rem solid var(--brand)`; warnings use a left warning border;
errors use a left danger border. Do not add decorative cards around plain
headings or paragraphs.

- [ ] **Step 3: Add final responsive rules**

In `app.css`:

```css
@media (max-width: 64rem) {
  .app-shell main {
    width: min(100% - 2rem, 68rem);
  }

  .app-shell main.manual-setup-page {
    width: min(100% - 2rem, 58rem);
  }
}

@media (max-width: 48rem) {
  .topbar-inner {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .main-navigation {
    grid-column: 1 / -1;
    grid-row: 2;
  }

  .section-heading {
    display: grid;
    align-items: start;
  }

  .section-action {
    justify-self: start;
  }

  .target-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 30rem) {
  .topbar-inner,
  .app-shell main,
  .app-shell main.manual-setup-page {
    width: calc(100% - 1.25rem);
  }
}
```

The explicit `.app-shell main.manual-setup-page` rules are required so the
generic `main` selector cannot widen Manual setup again at `1024px`. Keep
existing reduced-motion and forced-colors rules.

- [ ] **Step 4: Run the complete frontend suite**

Run:

```powershell
npm test --workspace apps/desktop
npm run build --workspace apps/desktop
```

Expected: all frontend tests and production build pass; all four CSS files
pass no-gradient and page token-only contracts.

- [ ] **Step 5: Perform real visual inspection**

Launch:

```powershell
npm start
```

Inspect and capture Home, ChatGPT/OpenAI manual setup, Claude manual setup,
CLI ready, CLI unavailable, History, and Result at:

```text
1920×1080
1366×768
1024px wide
480px wide
```

For each view verify:

- no horizontal scroll;
- topbar controls remain visible;
- section title, metadata, and action stay grouped;
- no oversized empty hero region;
- model and effort columns are balanced;
- status changes do not shift the main action unexpectedly;
- light and dark themes remain readable;
- keyboard focus is visible.

Save screenshots under the existing ignored `.superpowers/sdd/visual-qa/`
directory; do not commit machine screenshots unless the user requests them.

- [ ] **Step 6: Commit remaining UI migration**

```powershell
git add apps/desktop/src/pages/CliRunPage.css apps/desktop/src/pages/ResultsHistory.css apps/desktop/src/styles/app.css apps/desktop/src/test/accessibility.test.tsx
git commit -m "style: unify radar pages and responsive layout"
```

---

### Task 6: Seal v0.2.2 Candidate Contracts and Packages

**Files:**

- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `crates/ability-core/Cargo.toml`
- Modify: `crates/ability-adapters/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `scripts/validate-repository.mjs`
- Modify: `scripts/repository-contracts.test.mjs`
- Modify: `scripts/package-portable.test.mjs`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/ISSUE_TEMPLATE/bug.yml`
- Modify: `README.md`
- Modify: `docs/troubleshooting.md`
- Modify: `docs/privacy.md`
- Modify: `docs/test-matrix.md`
- Modify: `docs/release-checklist.md`
- Modify: `site/index.html`
- Regenerate: `docs/licenses/npm-dependencies.json`
- Regenerate: `docs/licenses/rust-dependencies.json`

**Interfaces:**

- Consumes: all verified v0.2.2 functionality.
- Produces: one internally consistent candidate version and three unsigned
  Windows artifacts, still held from public release.

- [ ] **Step 1: Update failing version contract tests first**

In `repository-contracts.test.mjs`, change the sealed test name and expected
message to `0.2.2`, then mutate the fixture to `0.2.1`:

```js
test("all first-party manifests require version 0.2.2", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "package.json"), (source) => {
      const manifest = JSON.parse(source);
      manifest.version = "0.2.1";
      return `${JSON.stringify(manifest, null, 2)}\n`;
    });
  });
  assertRejected(result, /package\.json version must be 0\.2\.2/i);
});
```

Change current-release negative tests, CTA tests, bug example, release body,
and artifact paths from `v0.2.1`/`0.2.1` to `v0.2.2`/`0.2.2`.

In `package-portable.test.mjs`, update fixtures representing the current
valid archive to `0.2.2`; keep deliberately malformed semantic versions
malformed relative to `0.2.2`.

- [ ] **Step 2: Run contracts and confirm production files are stale**

Run:

```powershell
node --test scripts/repository-contracts.test.mjs
node --test scripts/package-portable.test.mjs
```

Expected: tests fail because manifests, validator, workflows, documentation,
and site still declare v0.2.1.

- [ ] **Step 3: Update all first-party versions**

Set exactly `0.2.2` in:

```text
package.json
apps/desktop/package.json
apps/desktop/src-tauri/Cargo.toml
apps/desktop/src-tauri/tauri.conf.json
crates/ability-core/Cargo.toml
crates/ability-adapters/Cargo.toml
```

Update root/workspace entries in `package-lock.json` and first-party package
entries in `Cargo.lock`. Do not alter unrelated dependency versions such as
`windows-link 0.2.1`.

Set:

```js
const expectedVersion = "0.2.2";
```

in `scripts/validate-repository.mjs`, then update its exact artifact,
release-body, pending banner, inactive CTA, and bug-placeholder contracts to
v0.2.2.

- [ ] **Step 4: Update workflows and truthful candidate copy**

Change CI artifact path to:

```text
target/debug/bundle/nsis/ability-radar_0.2.2_x64-setup.exe
```

Change release body to:

```text
Windows 10/11 x64 v0.2.2 预览版。
```

Update README, troubleshooting, privacy, test matrix, release checklist,
site title/banner/disabled CTA, and bug placeholder to v0.2.2. Preserve these
truths:

- candidate/pending;
- public download unavailable;
- unsigned;
- no automatic updates;
- clean Windows 10/11 x64 validation still required;
- true CLI runs consume the runner's subscription;
- client selector identification is local and may fall back to manual.

The disabled site CTA must remain a non-link:

```html
<span class="button disabled" id="release-link" aria-disabled="true">
  v0.2.2 下载待开放
</span>
```

- [ ] **Step 5: Regenerate lock and license metadata**

Run:

```powershell
cargo check --workspace --locked
npm install --package-lock-only --ignore-scripts
npm run licenses:generate
```

Expected: only first-party version/dependency-edge and generated license
metadata changes occur. Review:

```powershell
git diff -- package-lock.json Cargo.lock docs/licenses
```

Reject any unexplained dependency upgrade.

- [ ] **Step 6: Run the full release gate**

Run:

```powershell
npm run validate:repository
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm test
npm audit --ignore-scripts
cargo audit
```

Expected: all pass; no real provider is invoked.

- [ ] **Step 7: Build candidate artifacts**

Run:

```powershell
npm run tauri -- build
npm run package:portable:from-build
```

Expected files:

```text
target/release/bundle/nsis/ability-radar_0.2.2_x64-setup.exe
target/release/bundle/msi/ability-radar_0.2.2_x64_en-US.msi
target/release/bundle/portable/ability-radar_0.2.2_windows-x64-portable.zip
```

Generate and record SHA-256 values without committing binaries. Verify the
portable ZIP by extracting to a fresh temporary directory and running the
repository’s existing portable validation flow.

- [ ] **Step 8: Commit v0.2.2 integration**

```powershell
git add package.json package-lock.json apps/desktop/package.json apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json crates/ability-core/Cargo.toml crates/ability-adapters/Cargo.toml Cargo.lock scripts/validate-repository.mjs scripts/repository-contracts.test.mjs scripts/package-portable.test.mjs .github/workflows/ci.yml .github/workflows/release.yml .github/ISSUE_TEMPLATE/bug.yml README.md docs/troubleshooting.md docs/privacy.md docs/test-matrix.md docs/release-checklist.md site/index.html docs/licenses/npm-dependencies.json docs/licenses/rust-dependencies.json
git commit -m "chore: prepare v0.2.2 Windows candidate"
```

- [ ] **Step 9: Reopen the final source build for user acceptance**

Run:

```powershell
npm start
```

Expected: the final v0.2.2 source application opens, detects
`codex-cli 0.142.5`, shows the refined UI, and allows client identification
fallback and manual testing. Do not push, publish a release, or activate
download links without a separate user instruction and clean Windows
acceptance.
