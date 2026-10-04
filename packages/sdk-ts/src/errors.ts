/** Fehlerantwort des Servers als RFC-9457-Problem (PROTO-011). */
export interface Problem {
  type?: string
  title?: string
  status?: number
  detail?: string
  code?: string
  [key: string]: unknown
}

export class BetonError extends Error {
  readonly status: number
  readonly problem: Problem

  constructor(status: number, problem: Problem) {
    super(problem.detail ?? problem.title ?? `HTTP ${status}`)
    this.name = 'BetonError'
    this.status = status
    this.problem = problem
  }

  get code(): string | undefined {
    return this.problem.code
  }
}
