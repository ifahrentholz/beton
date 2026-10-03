import raw from './features.json'

/** Ein Feature aus der Spec (generiert von scripts/gen_feature_catalog.py). */
export type Feature = {
  id: string
  prefix: string
  title: string
  milestone: string
  priority: 'Must' | 'Should' | 'Could'
  chapter: string
  chapterTitle: string
  anchor: string
  summary: string
  acs: string[]
}

export const features = raw as Feature[]
export const featureById = new Map(features.map((f) => [f.id, f]))

export const SPEC_BASE = 'https://github.com/ifahrentholz/beton/blob/main/docs/spec'
export const specUrl = (f: Feature) => `${SPEC_BASE}/${f.chapter}#${f.anchor}`
