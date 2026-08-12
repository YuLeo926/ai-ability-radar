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
    topEnv: {},
    topEnvDeclaration: "absent",
    topEnvValid: true,
    hasConcurrency: false,
    jobs: new Map(),
  };
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  let inJobs = false;
  let topSection;
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
      topSection = undefined;
      const pair = keyValue(text);
      if (pair?.[0] === "env") {
        workflow.topEnvDeclaration =
          pair[1] === "" ? "block" : "unsupported";
        workflow.topEnvValid = pair[1] === "";
        if (workflow.topEnvValid) topSection = "env";
      }
      if (text.startsWith("permissions:")) {
        workflow.topPermissions = scalar(text.slice("permissions:".length));
      } else if (text === "concurrency:") {
        workflow.hasConcurrency = true;
      }
      continue;
    }
    if (!inJobs) {
      if (indent === 2 && topSection === "env") {
        const pair = keyValue(text);
        if (!pair || pair[0] === "<<") workflow.topEnvValid = false;
        else workflow.topEnv[pair[0]] = pair[1];
      }
      continue;
    }

    if (indent === 2) {
      const pair = keyValue(text);
      if (!pair || pair[1] !== "") continue;
      job = {
        id: pair[0],
        permissions: {},
        permissionsDeclaration: "absent",
        permissionsValid: true,
        env: {},
        envDeclaration: "absent",
        envValid: true,
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
      const pair = keyValue(text);
      if (pair?.[0] === "permissions") {
        job.permissionsDeclaration =
          pair[1] === "" ? "block" : "unsupported";
        job.permissionsValid = pair[1] === "";
        jobSection = job.permissionsValid ? "permissions" : undefined;
      } else if (pair?.[0] === "env") {
        job.envDeclaration = pair[1] === "" ? "block" : "unsupported";
        job.envValid = pair[1] === "";
        jobSection = job.envValid ? "env" : undefined;
      } else if (pair?.[0] === "strategy") {
        job.strategy = pair[1] === "" ? {} : { unsupported: pair[1] };
        jobSection = pair[1] === "" ? "strategy" : undefined;
      } else if (text === "steps:") jobSection = "steps";
      else {
        jobSection = undefined;
        if (pair?.[0] === "timeout-minutes") job.timeoutMinutes = pair[1];
        else if (pair) job[pair[0]] = pair[1];
      }
      continue;
    }

    if (indent === 6 && jobSection === "permissions") {
      const pair = keyValue(text);
      if (!pair || pair[0] === "<<") job.permissionsValid = false;
      else job.permissions[pair[0]] = pair[1];
      continue;
    }
    if (indent === 6 && jobSection === "env") {
      const pair = keyValue(text);
      if (!pair || pair[0] === "<<") job.envValid = false;
      else job.env[pair[0]] = pair[1];
      continue;
    }
    if (
      indent === 6 &&
      (jobSection === "strategy" || jobSection === "strategy-matrix")
    ) {
      const pair = keyValue(text);
      if (!pair || pair[0] === "<<") {
        job.strategy.unsupported = text;
      } else if (pair[0] === "matrix" && pair[1] === "") {
        job.strategy.matrix = {};
        jobSection = "strategy-matrix";
      } else {
        job.strategy[pair[0]] = pair[1];
        jobSection = "strategy";
      }
      continue;
    }
    if (indent === 8 && jobSection === "strategy-matrix") {
      const pair = keyValue(text);
      if (!pair || pair[0] === "<<") job.strategy.unsupported = text;
      else job.strategy.matrix[pair[0]] = pair[1];
      continue;
    }
    if (indent === 6 && jobSection === "steps" && text.startsWith("-")) {
      step = {
        name: "",
        uses: "",
        usesComment: "",
        with: {},
        withDeclaration: "absent",
        withValid: true,
        env: {},
        envDeclaration: "absent",
        envValid: true,
        run: "",
      };
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
      if (key === "with") {
        step.withDeclaration = value === "" ? "block" : "unsupported";
        step.withValid = value === "";
        stepSection = step.withValid ? "with" : undefined;
      } else if (key === "env") {
        step.envDeclaration = value === "" ? "block" : "unsupported";
        step.envValid = value === "";
        stepSection = step.envValid ? "env" : undefined;
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
      if (!pair || pair[0] === "<<") {
        step[`${stepSection}Valid`] = false;
        continue;
      }
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
