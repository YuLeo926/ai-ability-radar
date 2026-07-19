import { useState } from "react";
import type { TargetKind } from "../api/backend";
import {
  effortOptionsFor,
  reasoningEffortError,
} from "../domain/reasoningEffort";

const CUSTOM_VALUE = "__custom__";
const CUSTOM_REQUIRED_ERROR = "请填写自定义推理档位";

export function ReasoningEffortField({
  emptyLabel,
  id,
  kind,
  label,
  onChange,
  onValidationChange,
  value,
}: {
  emptyLabel: string;
  id: string;
  kind: TargetKind;
  label: string;
  onChange(value: string): void;
  onValidationChange(error: string | null): void;
  value: string;
}) {
  const options = effortOptionsFor(kind);
  const preset = options.some((option) => option.value === value);
  const [customMode, setCustomMode] = useState(Boolean(value) && !preset);
  const custom = customMode || (Boolean(value) && !preset);
  const error = custom
    ? value.trim()
      ? reasoningEffortError(kind, value)
      : CUSTOM_REQUIRED_ERROR
    : null;
  const errorId = `${id}-error`;

  return (
    <div className="field reasoning-effort-field">
      <label htmlFor={id}>{label}</label>
      <select
        aria-describedby={error ? errorId : undefined}
        aria-invalid={error ? "true" : undefined}
        id={id}
        onChange={(event) => {
          const next = event.target.value;
          if (next === CUSTOM_VALUE) {
            setCustomMode(true);
            onChange("");
            onValidationChange(CUSTOM_REQUIRED_ERROR);
          } else {
            setCustomMode(false);
            onChange(next);
            onValidationChange(null);
          }
        }}
        value={custom ? CUSTOM_VALUE : value}
      >
        <option value="">{emptyLabel}</option>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
        <option value={CUSTOM_VALUE}>其他 / 按界面原样填写</option>
      </select>
      {custom ? (
        <label className="reasoning-custom">
          <span>按界面原样填写</span>
          <input
            autoComplete="off"
            onChange={(event) => {
              const next = event.target.value;
              onChange(next);
              onValidationChange(
                next.trim()
                  ? reasoningEffortError(kind, next)
                  : CUSTOM_REQUIRED_ERROR,
              );
            }}
            value={value}
          />
        </label>
      ) : null}
      {error ? (
        <p className="form-error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
      <small className="hint">
        可用档位取决于模型、客户端版本和账户权限。
      </small>
    </div>
  );
}
