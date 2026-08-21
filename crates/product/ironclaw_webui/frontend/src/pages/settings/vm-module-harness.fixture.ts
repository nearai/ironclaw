import {
  deepStrictEqual,
  strictEqual,
} from "node:assert/strict"

const value = "still here";

export function readValue() {
  return value;
}

const base = 41;

export interface AnswerMetadata {
  source: string;
}

export type Answer = number;

export const answer = base + 1;

function readAnswer() {
  return answer;
}

export { readAnswer as getAnswer };
