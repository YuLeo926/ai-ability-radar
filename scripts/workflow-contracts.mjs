function leadingSpaces(line) {
  return line.match(/^ */)[0].length;
}

function splitYamlComment(line) {
  let single = false;
  let double = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (character === "'" && !double) single = !single;
    if (character === '"' && !single && line[index - 1] !== "\\") double = !double;
    if (
      character === "#" &&
      !single &&
      !double &&
      (index === 0 || /\s/.test(line[index - 1]))
    ) {
      return {
        content: line.slice(0, index).trimEnd(),
        comment: line.slice(index + 1).trim(),
      };
    }
  }
  return { content: line.trimEnd(), comment: "" };
}

function scalar(value) {
  const trimmed = value.trim();
  if (
    trimmed.length >= 2 &&
    ((trimmed.startsWith('"') && trimmed.endsWith('"')) ||
      (trimmed.startsWith("'") && trimmed.endsWith("'")))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function keyValue(text) {
  const match = text.match(/^([A-Za-z0-9_-]+):(?:\s*(.*))?$/);
  return match ? [match[1], scalar(match[2] ?? "")] : undefined;
}

function isBlockScalar(value) {
  return /^[|>](?:(?:[+-][1-9]?)|(?:[1-9][+-]?))?$/.test(value);
}

function cleanRun(lines) {
  return lines
    .filter((line) => !line.trimStart().startsWith("#"))
    .join("\n")
    .trim();
}

export function parseWorkflow(source) {
  const workflow = {
    topPermissions: undefined,
    hasConcurrency: false,
    jobs: new Map(),
  };
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  let inJobs = false;
  let job;
  let jobSection;
  let step;
  let stepSection;
  let block;

  function finishBlock() {
    if (!block) return;
    const value = cleanRun(block.lines);
    if (block.target === "run") block.step.run = value;
    else block.step.with[block.key] = value;
    block = undefined;
  }

  for (const rawLine of lines) {
    const indent = leadingSpaces(rawLine);
    if (block && rawLine.trim() === "") {
      block.lines.push("");
      continue;
    }

    if (block && indent > block.indent) {
      block.lines.push(rawLine.slice(block.indent + 2));
      continue;
    }
    finishBlock();

    const { content, comment } = splitYamlComment(rawLine);
    const text = content.trim();
    if (!text) continue;

    if (indent === 0) {
      job = undefined;
      step = undefined;
      jobSection = undefined;
      stepSection = undefined;
      inJobs = text === "jobs:";
      if (text.startsWith("permissions:")) {
        workflow.topPermissions = scalar(text.slice("permissions:".length));
      } else if (text === "concurrency:") {
        workflow.hasConcurrency = true;
      }
      continue;
    }
    if (!inJobs) continue;

    if (indent === 2) {
      const pair = keyValue(text);
      if (!pair || pair[1] !== "") continue;
      job = {
        id: pair[0],
        permissions: {},
        env: {},
        timeoutMinutes: undefined,
        steps: [],
      };
      workflow.jobs.set(job.id, job);
      step = undefined;
      jobSection = undefined;
      continue;
    }
    if (!job) continue;

    if (indent === 4) {
      step = undefined;
      stepSection = undefined;
      if (text === "permissions:") jobSection = "permissions";
      else if (text === "env:") jobSection = "env";
      else if (text === "steps:") jobSection = "steps";
      else {
        jobSection = undefined;
        const pair = keyValue(text);
        if (pair?.[0] === "timeout-minutes") job.timeoutMinutes = pair[1];
      }
      continue;
    }

    if (indent === 6 && jobSection === "permissions") {
      const pair = keyValue(text);
      if (pair) job.permissions[pair[0]] = pair[1];
      continue;
    }
    if (indent === 6 && jobSection === "env") {
      const pair = keyValue(text);
      if (pair) job.env[pair[0]] = pair[1];
      continue;
    }
    if (indent === 6 && jobSection === "steps" && text.startsWith("-")) {
      step = { name: "", uses: "", usesComment: "", with: {}, env: {}, run: "" };
      job.steps.push(step);
      stepSection = undefined;
      const pair = keyValue(text.slice(1).trim());
      if (pair) {
        step[pair[0]] = pair[1];
        if (pair[0] === "uses") step.usesComment = comment;
      }
      continue;
    }
    if (!step) continue;

    if (indent === 8) {
      const pair = keyValue(text);
      if (!pair) continue;
      const [key, value] = pair;
      if (key === "with" && value === "") {
        stepSection = "with";
      } else if (key === "env" && value === "") {
        stepSection = "env";
      } else {
        stepSection = undefined;
        if (key === "run" && isBlockScalar(value)) {
          block = { target: "run", step, indent, lines: [] };
        } else {
          step[key] = value;
          if (key === "uses") step.usesComment = comment;
        }
      }
      continue;
    }

    if (indent === 10 && (stepSection === "with" || stepSection === "env")) {
      const pair = keyValue(text);
      if (!pair) continue;
      const [key, value] = pair;
      if (stepSection === "with" && isBlockScalar(value)) {
        block = { target: "with", key, step, indent, lines: [] };
      } else {
        step[stepSection][key] = value;
      }
    }
  }
  finishBlock();
  return workflow;
}

export function actionSteps(workflow, action) {
  const prefix = `${action}@`;
  return [...workflow.jobs.values()]
    .flatMap((job) => job.steps)
    .filter((step) => step.uses.startsWith(prefix));
}

export function runSteps(workflow) {
  return [...workflow.jobs.values()].flatMap((job) =>
    job.steps.filter((step) => step.run));
}

export function hasRunCommand(job, pattern) {
  return job?.steps.some((step) => pattern.test(step.run)) ?? false;
}

export function exactPermissions(job, expected) {
  if (!job) return false;
  const actualEntries = Object.entries(job.permissions).sort();
  const expectedEntries = Object.entries(expected).sort();
  return JSON.stringify(actualEntries) === JSON.stringify(expectedEntries);
}
