"""Score lecture_pipeline runs against a reviewed lecture reference.

Usage:
    python scripts/lecture_eval.py <lecture dir>... [--output evaluation/results/<date>-lecture-eval.json]

A lecture dir holds reference.json (lecture-reference-v1) and runs/<name>/ with
lecture-pipeline.json and transcript.txt. Segment IDs only compare between runs that share the
transcript the reference was written from, so any other run is refused. The printed report
names facts and notices; the written summary carries numbers only, no lecture content.
"""
import argparse
import json
from pathlib import Path
import re
import sys

NOTICE_KINDS = {'exam', 'assignment', 'announcement'}
NOTICE_STATUSES = {'scheduled', 'cancelled'}
IMPORTANCE = {'key', 'normal'}
REVIEW_STATUSES = {'draft', 'reviewed'}
REQUIRED = ['schema_version', 'lecture_id', 'course', 'date', 'permission', 'review', 'transcript_run',
            'facts', 'notices', 'code', 'terms']
AFTER_RECORDING_LIMIT = 300.0
KEY_FACT_RECALL = 0.95


def _segments_ok(value):
    return isinstance(value, list) and value and all(isinstance(item, str) and re.fullmatch(r's\d+', item)
                                                     for item in value)


def reference_problems(reference):
    """Every way the reference breaks lecture-reference-v1; empty when it is usable."""
    problems = [f'missing {key}' for key in REQUIRED if key not in reference]
    if problems:
        return problems
    if reference['schema_version'] != 'lecture-reference-v1':
        problems.append('schema_version is not lecture-reference-v1')
    permission = reference['permission']
    for key in ['granted_by', 'scope', 'confirmed_on']:
        if not str(permission.get(key) or '').strip():
            problems.append(f'permission.{key} is empty: without permission the lecture stays out')
    if reference['review'].get('status') not in REVIEW_STATUSES:
        problems.append('review.status must be draft or reviewed')
    seen = set()
    for fact in reference['facts']:
        if fact.get('id') in seen:
            problems.append(f'duplicate fact id {fact.get("id")}')
        seen.add(fact.get('id'))
        if fact.get('importance') not in IMPORTANCE:
            problems.append(f'fact {fact.get("id")} importance must be key or normal')
        if not _segments_ok(fact.get('segments')):
            problems.append(f'fact {fact.get("id")} needs segments')
    for index, notice in enumerate(reference['notices']):
        if notice.get('kind') not in NOTICE_KINDS:
            problems.append(f'notice {index} kind must be one of {sorted(NOTICE_KINDS)}')
        if notice.get('status') not in NOTICE_STATUSES:
            problems.append(f'notice {index} status must be scheduled or cancelled')
        if not _segments_ok(notice.get('segments')):
            problems.append(f'notice {index} needs segments')
    for index, item in enumerate(reference['code']):
        if not str(item.get('code') or '').strip() or not _segments_ok(item.get('segments')):
            problems.append(f'code {index} needs code and segments')
    for index, term in enumerate(reference['terms']):
        if not str(term.get('correct') or '').strip() or not _segments_ok(term.get('segments')):
            problems.append(f'term {index} needs a correct form and segments')
    return problems


def _squeeze(text):
    return ' '.join(str(text).split()).lower()


def _ratio(found, total):
    return None if total == 0 else found / total


def _note(run, name):
    note = run.get(name)
    return note['value'] if note else None


def _notice_scores(reference, note):
    notices = note['notices'] if note else []
    matched = {}
    for position, expected in enumerate(reference['notices']):
        for index, actual in enumerate(notices):
            if index in matched.values():
                continue
            if actual['kind'] == expected['kind'] and set(actual['source_refs']) & set(expected['segments']):
                matched[position] = index
                break
    status_errors = sum(1 for position, index in matched.items()
                        if notices[index]['status'] != reference['notices'][position]['status'])
    date_errors = 0
    for position, index in matched.items():
        written = notices[index].get('date_text')
        if written is not None and _squeeze(written) != _squeeze(reference['notices'][position].get('date_text') or ''):
            date_errors += 1
    invented = [index for index in range(len(notices)) if index not in matched.values()]
    return {
        'expected': len(reference['notices']),
        'written': len(notices),
        'recall': _ratio(len(matched), len(reference['notices'])),
        'missed': [position for position in range(len(reference['notices'])) if position not in matched],
        'invented': invented,
        'status_errors': status_errors,
        'date_errors': date_errors,
    }


def _code_scores(reference, notes):
    expected = {_squeeze(item['code']) for item in reference['code']}
    written = {_squeeze(item['code']) for note in notes if note for item in note['code']}
    hits = expected & written
    return {
        'expected': len(expected),
        'written': len(written),
        'recall': _ratio(len(hits), len(expected)),
        'precision': _ratio(len(hits), len(written)),
    }


def _term_scores(reference, transcript, notes):
    note_text = _squeeze(json.dumps([note for note in notes if note], ensure_ascii=False))
    transcript_text = _squeeze(transcript)
    terms = reference['terms']
    return {
        'terms': len(terms),
        'correct_in_transcript': _ratio(sum(1 for term in terms if _squeeze(term['correct']) in transcript_text),
                                        len(terms)),
        'correct_in_notes': _ratio(sum(1 for term in terms if _squeeze(term['correct']) in note_text), len(terms)),
    }


def _fact_scores(reference, note):
    """A fact counts as found when a concept, example or term of the note cites one of its
    segments. This is a proxy: the final call on each fact is a reviewer's."""
    cited = set()
    if note:
        for section in ['concepts', 'examples', 'terms']:
            for item in note.get(section, []):
                cited.update(item['source_refs'])
    scores = {}
    for importance in ['key', 'normal']:
        facts = [fact for fact in reference['facts'] if fact['importance'] == importance]
        missed = [fact['id'] for fact in facts if not set(fact['segments']) & cited]
        scores[f'{importance}_recall'] = _ratio(len(facts) - len(missed), len(facts))
        scores[f'{importance}_missed'] = missed
    return scores


def score(reference, run, transcript, reference_transcript):
    """Scores one run. `transcript` is the run's, `reference_transcript` the one the
    reference's segment IDs come from; they must be the same text."""
    if transcript.strip() != reference_transcript.strip():
        raise ValueError('the run has a different transcript; its segment IDs do not match the reference')
    five = _note(run, 'five_minute_note')
    precise = _note(run, 'precise_note')
    return {
        'mode': run.get('mode'),
        'after_recording_seconds': run.get('after_recording_seconds'),
        'notices': _notice_scores(reference, five or precise),
        'code': _code_scores(reference, [five, precise]),
        'terms': _term_scores(reference, transcript, [five, precise]),
        'facts': {'five_minute_note': _fact_scores(reference, five), 'precise_note': _fact_scores(reference, precise)},
    }


def verdict(scores, power):
    """Pass or fail per criterion of the v1 standard. Time is judged only for real
    recordings on AC; `None` means not judged."""
    notices = scores['notices']
    facts = scores['facts']
    key_recalls = [facts[name]['key_recall'] for name in ['five_minute_note', 'precise_note']]
    checks = {
        'key_facts': all(value is None or value >= KEY_FACT_RECALL for value in key_recalls),
        'notices': notices['recall'] in (None, 1.0) and not notices['invented']
        and notices['status_errors'] == 0 and notices['date_errors'] == 0,
        'after_recording': None,
    }
    if scores['mode'] == 'recording' and power and power.get('ac_power') is True \
            and scores['after_recording_seconds'] is not None:
        checks['after_recording'] = scores['after_recording_seconds'] <= AFTER_RECORDING_LIMIT
    return {'checks': checks, 'passed': all(value is not False for value in checks.values())}


def _power(run_dir):
    """The power state the run recorded at its end, if any; AC must hold for the whole run."""
    states = []
    for name in ['power-start.json', 'power-end.json']:
        path = run_dir / name
        if path.exists():
            states.append(json.loads(path.read_text('utf-8-sig')).get('power', {}))
    if not states:
        return None
    return {'ac_power': all(state.get('ac_power') is True for state in states)}


def score_lecture(lecture_dir):
    lecture_dir = Path(lecture_dir)
    reference = json.loads((lecture_dir / 'reference.json').read_text('utf-8'))
    problems = reference_problems(reference)
    if problems:
        raise ValueError(f'{lecture_dir}: ' + '; '.join(problems))
    reference_transcript = (lecture_dir / 'runs' / reference['transcript_run'] / 'transcript.txt').read_text('utf-8')
    runs = []
    for run_dir in sorted(path for path in (lecture_dir / 'runs').iterdir() if path.is_dir()):
        report = run_dir / 'lecture-pipeline.json'
        if not report.exists():
            continue
        transcript = (run_dir / 'transcript.txt').read_text('utf-8')
        run = json.loads(report.read_text('utf-8'))
        try:
            scores = score(reference, run, transcript, reference_transcript)
        except ValueError as error:
            runs.append({'run': run_dir.name, 'skipped': str(error)})
            continue
        runs.append({'run': run_dir.name, 'scores': scores, 'verdict': verdict(scores, _power(run_dir))})
    return {'lecture_id': reference['lecture_id'], 'course': reference['course'],
            'review': reference['review']['status'], 'runs': runs}


def summary(results):
    """Numbers only: no course names, fact texts or notice contents."""
    lectures = []
    for result in results:
        runs = []
        for run in result['runs']:
            if 'skipped' in run:
                runs.append({'run': run['run'], 'skipped': True})
                continue
            scores = run['scores']
            runs.append({
                'run': run['run'],
                'mode': scores['mode'],
                'after_recording_seconds': scores['after_recording_seconds'],
                'notices': {key: scores['notices'][key] for key in
                            ['expected', 'written', 'recall', 'status_errors', 'date_errors']}
                | {'invented': len(scores['notices']['invented'])},
                'code': scores['code'],
                'terms': scores['terms'],
                'facts': {name: {key: value for key, value in values.items() if key.endswith('recall')}
                          for name, values in scores['facts'].items()},
                'verdict': run['verdict'],
            })
        lectures.append({'lecture_id': result['lecture_id'], 'review': result['review'], 'runs': runs})
    return {'reference_schema': 'lecture-reference-v1', 'lectures': lectures}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('lectures', nargs='+')
    parser.add_argument('--output')
    arguments = parser.parse_args(argv)
    results = [score_lecture(path) for path in arguments.lectures]
    for result in results:
        print(f"{result['lecture_id']} (reference {result['review']})")
        for run in result['runs']:
            if 'skipped' in run:
                print(f"  {run['run']}: skipped, {run['skipped']}")
                continue
            scores, checks = run['scores'], run['verdict']['checks']
            facts = scores['facts']
            print(f"  {run['run']} ({scores['mode']}): passed {run['verdict']['passed']} {checks}")
            print(f"    key facts: five-minute {facts['five_minute_note']['key_recall']} "
                  f"missed {facts['five_minute_note']['key_missed']}, precise {facts['precise_note']['key_recall']} "
                  f"missed {facts['precise_note']['key_missed']}")
            print(f"    notices: {scores['notices']}")
            print(f"    code: {scores['code']}  terms: {scores['terms']}")
    if arguments.output:
        Path(arguments.output).write_text(json.dumps(summary(results), ensure_ascii=False, indent=1) + '\n', 'utf-8')
    return 0


if __name__ == '__main__':
    sys.exit(main())
