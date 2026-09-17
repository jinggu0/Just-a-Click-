"""Prototype meeting contract. Structural/lexical checks are not semantic proof."""
import copy
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / 'schemas/meeting-summary-v1.json').read_text('utf-8'))
STRICT_INSTRUCTION = (
    '\nmeeting-summary-v1 스키마를 따른다. 각 actions 항목에는 content, owner, deadline, '
    'source_refs를 반드시 모두 포함한다. 미정인 담당자·기한은 null이다. 명시된 값은 '
    '근거 원문의 표기를 그대로 사용한다. 실제로 맡기로 한 후속 작업만 actions에 넣고, '
    '미측정·미결 상태는 open_questions에 넣는다. 중요한 사실·부정·취소·시간 범위를 '
    '보존하며 원문 문장을 그대로 반복하지 말고 간결하게 요약한다. '
    '제공된 JSON 스키마와 출처 ID만 사용한다.'
)


def parse_segments(transcript):
    segments = {}
    for line in transcript.splitlines():
        match = re.fullmatch(r'\[(s\d+) [0-9:]+\]\s*(.+)', line)
        if not match or match[1] in segments:
            raise ValueError('Malformed or duplicate transcript segment')
        segments[match[1]] = match[2]
    if not segments:
        raise ValueError('Empty transcript')
    return segments


def schema_for_sources(segments):
    schema = copy.deepcopy(SCHEMA)
    for section in ['decisions', 'actions', 'open_questions']:
        schema['properties'][section]['items']['properties']['source_refs']['items']['enum'] = list(segments)
    return schema


def _validate(value, schema, path='$'):
    """Only the keywords used by our checked-in schema; not a general JSON Schema engine."""
    supported = {'type', 'const', 'enum', 'minLength', 'minItems', 'items',
                 'properties', 'required', 'additionalProperties'}
    if set(schema) - supported:
        raise ValueError('Unsupported schema keyword')
    allowed = schema['type'] if isinstance(schema['type'], list) else [schema['type']]
    kind = {dict: 'object', list: 'array', str: 'string', type(None): 'null'}.get(type(value))
    if kind not in allowed:
        raise ValueError(f'{path}: invalid type')
    if 'const' in schema and value != schema['const']:
        raise ValueError(f'{path}: invalid constant')
    if 'enum' in schema and value not in schema['enum']:
        raise ValueError(f'{path}: unknown source')
    if kind == 'string' and len(value.strip()) < schema.get('minLength', 0):
        raise ValueError(f'{path}: empty string')
    if kind == 'object':
        if set(schema.get('required', [])) - set(value):
            raise ValueError(f'{path}: missing required fields')
        properties = schema.get('properties', {})
        if schema.get('additionalProperties') is False and set(value) - set(properties):
            raise ValueError(f'{path}: unexpected fields')
        for key, item in value.items():
            _validate(item, properties[key], f'{path}.{key}')
    if kind == 'array':
        if len(value) < schema.get('minItems', 0):
            raise ValueError(f'{path}: empty references')
        for index, item in enumerate(value):
            _validate(item, schema['items'], f'{path}[{index}]')


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'Duplicate JSON field: {key}')
        result[key] = value
    return result


def validate_summary(content, segments, finish_reason='stop'):
    if finish_reason != 'stop':
        raise ValueError('Incomplete generation')
    result = json.loads(content, object_pairs_hook=_unique_object)
    _validate(result, schema_for_sources(segments))
    for section in ['decisions', 'actions', 'open_questions']:
        for item in result[section]:
            if len(set(item['source_refs'])) != len(item['source_refs']):
                raise ValueError('Duplicate source references')
    for action in result['actions']:
        cited = '\n'.join(segments[ref] for ref in action['source_refs'])
        for field in ['owner', 'deadline']:
            value = action[field]
            placeholder_deadline = field == 'deadline' and value in {'미정', '없음', '확인 필요'}
            if value is not None and (placeholder_deadline or value not in cited):
                raise ValueError(f'{field}: not a verbatim value in cited source')
    return result


def check_smoke_expectations(summary):
    """Fixture-only regressions, never a universal factual-quality score."""
    actions = summary['actions']
    return {
        'jimin_deadline_null': any(a['owner'] == '지민' and a['deadline'] is None
                                 and a['source_refs'] == ['s3'] for a in actions),
        'no_invented_deadline': all(a['deadline'] is None for a in actions),
        'gpu_state_in_open_questions': any('s6' in i['source_refs'] for i in summary['open_questions']),
        'gpu_state_not_an_action': all('s6' not in i['source_refs'] for i in actions),
        'required_fact_sources': {'s2', 's3', 's4', 's5', 's6'} <= {
            ref for section in ['decisions', 'actions', 'open_questions']
            for item in summary[section] for ref in item['source_refs']},
        'chatter_source_excluded': all('s7' not in i['source_refs']
            for section in ['decisions', 'actions', 'open_questions'] for i in summary[section])
    }
