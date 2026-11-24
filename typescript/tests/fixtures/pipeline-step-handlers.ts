import type { Step, StepResult } from '../../src/pipeline.js';

export class ConfiguredTestStep implements Step {
  readonly name: string;
  readonly description = 'A configured test step';
  readonly removable = true;
  readonly replaceable = true;

  constructor(config: Record<string, unknown>) {
    this.name = String(config['label']);
  }

  async execute(): Promise<StepResult> {
    return { action: 'continue' };
  }
}

export function createStep(config: Record<string, unknown>): Step {
  return new ConfiguredTestStep(config);
}

export const createArrowStep = (config: Record<string, unknown>): Step =>
  new ConfiguredTestStep(config);
