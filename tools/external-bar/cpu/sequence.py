"""Strict prefixes of the fixed nine-phase Lom fixture sequence; no native I/O."""
import importlib.util
from pathlib import Path
import re

spec = importlib.util.spec_from_file_location('static_records', Path(__file__).with_name('static.py'))
s = importlib.util.module_from_spec(spec)
spec.loader.exec_module(s)
PHASES = ('initial', 'establish', 'seconds', 'workspace', 'minute', 'rename',
          'restore', 'reestablish', 'seconds-again')
UPLOADS = (1, 3, 1, 1, 2, 1, 1, 3, 1)
ARMS = {f'{role}-{height}': (role, height) for role in ('full', 'regions') for height in (24, 512)}
GROUPS = ((0, 1, 2), (3, 4, 5), (6, 7, 8))


def counts(role, height, phase):
    tiled = role == 'regions' and phase not in (0, 5, 6)
    uploads = UPLOADS[phase] if tiled else 1
    return dict(placements=str(3 if tiled else 1), uploads=str(uploads),
                upload_bytes=str((640 * uploads if tiled else 1920) * height * 4), regions_disabled='0')


def number(value):
    if re.fullmatch('0|[1-9][0-9]*', value) is None or int(value) > 2**64 - 1:
        raise ValueError('noncanonical sequence integer')
    return int(value)


def state(log, arm, *, final=False):
    if arm not in ARMS:
        raise ValueError('unknown sequence arm')
    role, height = ARMS[arm]
    text = log.decode('utf-8')
    renderer = ('lom_renderer schema=1 renderer=vello-cpu source=owned-ring-fill gpu=not-used '
                f'diagnostic=cpu-region-sequence role={role}')
    if [line for line in text.splitlines() if 'lom_renderer' in line] != [renderer]:
        raise ValueError('wrong sequence renderer')
    if s.rows(text, 'lom_shell_transport') != [dict(schema='1', wire='9p2000.L', revision='10', epoch='1')]:
        raise ValueError('wrong sequence transport')
    declaration = dict(schema='1', source='fixture', role=role, phases='9', hold_ms='10000')
    if s.rows(text, 'lom_region_sequence') != [declaration]:
        raise ValueError('missing or duplicate fixture declaration')
    if any(s.rows(text, tag) for tag in ('lom_cadence', 'lom_cadence_sample', 'lom_cadence_final',
                                        'lom_checksum_policy', 'lom_work_stats')) or 'lom_gpu_admission' in text:
        raise ValueError('unexpected workload or renderer')
    if s.rows(text, 'sophia_shell_component') != [dict(schema='1', status='negotiated', slot='0', role='bar',
            connection_epoch='1', content_grant_epoch='1', revision='10', gpu_mode='denied',
            gpu_grant_epoch='0', device_major='0', device_minor='0')]:
        raise ValueError('expected one GPU-denied bar')
    heads = s.rows(text, 'sophia_live_native_head')
    ready = [row for row in heads if row.get('status') == 'ready']
    if len(ready) != 1:
        raise ValueError('expected one ready head')
    head = ready[0]
    if (head.keys() != {'schema', 'status', 'output', 'head', 'connector', 'connector_id', 'mode', 'refresh_millihz', 'mirrored'}
            or any(head.get(k) != v for k, v in dict(schema='2', status='ready', connector='DP-1',
                mode='1920x1080', refresh_millihz='60000', mirrored='false').items())
            or (not final and len(heads) != 1)):
        raise ValueError('wrong native head')
    for key in ('output', 'head', 'connector_id'):
        s.positive(head[key])
    indicators = s.rows(text, 'sophia_shell_indicator_state')
    if len(indicators) != 9:
        raise ValueError('live indicator feed changed')
    indicator_generation = indicators[0].get('indicator_generation')
    s.positive(indicator_generation)
    if indicators != [dict(schema='1', connection_epoch='1', indicator_generation=indicator_generation,
            output=head['output'], indicator=str(i+1), action=str(i+11), slot=str(i),
            state_bits='1' if i == 0 else '0', entries='9') for i in range(9)]:
        raise ValueError('live indicator feed differs from recipe')

    phases = s.rows(text, 'lom_region_phase')
    candidates = s.rows(text, 'lom_panel_candidate')
    regions = s.rows(text, 'lom_panel_regions')
    completed, installed = [], []
    for index, row in enumerate(phases):
        phase = index // 2
        if phase >= 9:
            raise ValueError('extra phase')
        event = 'installed' if index % 2 == 0 else 'presented'
        expected = dict(schema='1', event=event, source='fixture', role=role,
                        phase=str(phase), name=PHASES[phase])
        if event == 'presented':
            expected.update(candidate_generation=str(phase+1), **counts(role, height, phase))
            completed.append(row)
        else:
            installed.append(row)
        if row != expected:
            raise ValueError('phase order, identity or upload prediction differs')
    if not len(completed) <= len(candidates) <= len(installed) <= 9:
        raise ValueError('candidate count differs from phases')
    for phase, row in enumerate(candidates):
        expected = dict(schema='2', status='presented', connection_epoch='1', content_grant_epoch='1',
            output=head['output'], candidate_generation=str(phase+1), indicator_generation='0',
            width='1920', height=str(height), bytes=str(1920*height*4), checksum='unavailable', raster_source='rendered')
        if row.keys() != s.CANDIDATE or any(row[k] != v for k,v in expected.items()):
            raise ValueError('candidate is not the declared checksum-off fixture phase')
        s.positive(row['presentation_epoch'])
    expected_regions = []
    for phase in range(len(candidates)):
        count = counts(role, height, phase)
        if count['placements'] == '3':
            expected_regions.append(dict(schema='1', candidate_generation=str(phase+1),
                placements='3', uploads=count['uploads'], upload_bytes=count['upload_bytes'],
                retained=str(3-int(count['uploads']))))
    # The last candidate may have been printed just before its region row.
    if regions != expected_regions[:len(regions)] or len(regions) > len(expected_regions):
        raise ValueError('region counters differ')
    required_regions = sum(counts(role, height, phase)['placements'] == '3' for phase in range(len(completed)))
    if len(regions) < required_regions:
        raise ValueError('missing region counters for Presented phase')

    events = [line for line in text.splitlines() if line.startswith(('lom_renderer ', 'lom_region_sequence ',
        'lom_region_phase ', 'lom_panel_candidate ', 'lom_panel_regions ', 'lom_region_final '))]
    tags = [line.split()[0] for line in events]
    expected_tags = ['lom_renderer', 'lom_region_sequence']
    for phase in range(len(installed)):
        expected_tags.append('lom_region_phase')
        if phase < len(candidates):
            expected_tags.append('lom_panel_candidate')
            if any(r['candidate_generation'] == str(phase+1) for r in regions):
                expected_tags.append('lom_panel_regions')
        if phase < len(completed):
            expected_tags.append('lom_region_phase')
    finals = s.rows(text, 'lom_region_final')
    if finals:
        if finals != [dict(schema='1', source='fixture', role=role, status='complete', phases='9', presented='9')] or len(completed) != 9:
            raise ValueError('incomplete or duplicate sequence final')
        expected_tags.append('lom_region_final')
    if tags != expected_tags or (final and (len(completed) != 9 or not finals)):
        raise ValueError('sequence record ordering or completion differs')

    host = s.rows(text, 'sophia_live_shell_content')
    if not host or host[0] != dict(schema='1', status='outputs', facts_generation='1', outputs='1',
                                   connection_epoch='1', content_grant_epoch='1'):
        raise ValueError('output facts differ')
    if not 2*len(candidates) <= len(host)-1 <= 2*len(installed):
        raise ValueError('host candidate count differs')
    for index, row in enumerate(host[1:]):
        phase, presented = index // 2, index % 2 == 1
        if (row.keys() != (s.PRESENTED if presented else s.PREPARED) or row['schema'] != '1'
                or row['status'] != ('presented' if presented else 'prepared')
                or row['output'] != head['output'] or row['candidate_generation'] != str(phase+1)):
            raise ValueError('host phase or shape differs')
        for key,value in row.items():
            if key != 'status': number(value)
        # Retiring predecessors and accepted replacements can coexist until Presented.
        if (row['staging_bytes'] != '0' or int(row['resident_bytes']) < 1920*height*4
                or int(row['resident_bytes']) > 12*1024*1024 or int(row['retiring_bytes']) > 8*1024*1024
                or int(row['backing_bytes']) > int(row['resident_bytes'])+int(row['retiring_bytes'])):
            raise ValueError('host resource accounting differs')
        if presented and phase < len(candidates) and any(row[k] != candidates[phase][k]
                for k in ('presentation_epoch', 'connection_epoch', 'content_grant_epoch')):
            raise ValueError('host/client presentation join differs')
    lines = text.splitlines()
    host_positions = [i for i, line in enumerate(lines) if line.startswith('sophia_live_shell_content ')][1:]
    candidate_positions = [i for i, line in enumerate(lines) if line.startswith('lom_panel_candidate ')]
    phase_positions = [i for i, line in enumerate(lines) if line.startswith('lom_region_phase ')]
    for phase, position in enumerate(candidate_positions):
        if not phase_positions[2*phase] < host_positions[2*phase] < host_positions[2*phase+1] < position:
            raise ValueError('host/client phase ordering differs')
    sampling = []
    tag = 'sophia_native_composition_sampling'
    for line in text.splitlines():
        if tag not in line: continue
        prefix, record = line.split(tag, 1)
        if re.fullmatch(r'\d{4}-\d\d-\d\dT[0-9:.]+Z  INFO sophia_renderer_native_egl::gl: ', prefix) is None:
            raise ValueError('sampling prefix differs')
        sampling.extend(s.rows(tag+record, tag))
    if not sampling:
        raise ValueError('missing sampling proof')
    for row in sampling:
        allowed = [f'1920x{height}'] + ([f'640x{height}'] if role == 'regions' else [])
        expected = dict(schema='3', status='active', output=head['output'], head=head['head'],
            scene_generation=row.get('scene_generation'), requested='exact_nearest', effective='exact_nearest',
            alpha_mode='premultiplied', frame='1920x1080', source=row.get('source'), target=row.get('source'))
        if row != expected or row['source'] not in allowed:
            raise ValueError('sampling is not declared one-to-one geometry')
        s.positive(row['scene_generation'])
    return dict(head=head, completed=completed, installed=installed, candidates=candidates,
                host=host, indicators=indicators, final=bool(finals))


def snapshot(log, arm, phase):
    current = state(log, arm)
    if len(current['installed']) != phase+1 or len(current['completed']) != phase+1:
        raise ValueError('capture is outside its designated Presented hold')
    return dict(head=current['head'], candidate=current['candidates'][phase],
                phase=current['completed'][phase], indicators=current['indicators'])
