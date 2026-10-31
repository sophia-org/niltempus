"""One stable opaque Lom candidate; shell epochs and capture frames are distinct."""

import re

CANDIDATE = {'schema', 'status', 'connection_epoch', 'content_grant_epoch', 'output',
             'candidate_generation', 'presentation_epoch', 'indicator_generation',
             'width', 'height', 'bytes', 'checksum', 'raster_source'}
PRESENTED = {'schema', 'status', 'output', 'candidate_generation', 'presentation_epoch',
             'staging_bytes', 'resident_bytes', 'retiring_bytes', 'backing_bytes',
             'connection_epoch', 'content_grant_epoch'}
PREPARED = PRESENTED - {'presentation_epoch', 'connection_epoch', 'content_grant_epoch'}
STARTUP = {arm: 'lom_renderer schema=1 renderer=vello-cpu source=owned-ring-' + source +
           ' gpu=not-used diagnostic=cpu-reference'
           for arm, source in [('direct', 'fill'), ('copy', 'copy')]}
ARMS = {arm: (24, True) for arm in STARTUP}
for height in (24, 512):
    for policy in ('on', 'off'):
        arm = f'hash-{policy}-{height}'
        ARMS[arm] = (height, policy == 'on')
        STARTUP[arm] = STARTUP['direct'] if policy == 'on' else (
            'lom_renderer schema=1 renderer=vello-cpu source=owned-ring-fill gpu=not-used')


def rows(text, tag):
    result = []
    for line in text.splitlines():
        if tag not in line.split():
            continue
        if not line.startswith(tag + ' '):
            raise ValueError('embedded diagnostic record: ' + tag)
        row = {}
        for part in line[len(tag)+1:].split(' '):
            pair = part.split('=', 1)
            if len(pair) != 2 or not all(pair) or pair[0] in row:
                raise ValueError('malformed diagnostic record: ' + tag)
            row[pair[0]] = pair[1]
        result.append(row)
    return result


def positive(value):
    if re.fullmatch('[1-9][0-9]*', value) is None or int(value) > 2**64-1:
        raise ValueError('noncanonical positive identity')
    return int(value)


def snapshot(log, arm):
    if arm not in STARTUP:
        raise ValueError('expected a declared static arm')
    height, hashed = ARMS[arm]
    text = log.decode('utf-8')
    if [line for line in text.splitlines() if 'lom_renderer' in line] != [STARTUP[arm]]:
        raise ValueError('wrong diagnostic renderer')
    if rows(text, 'lom_shell_transport') != [dict(schema='1', wire='9p2000.L', revision='10', epoch='1')]:
        raise ValueError('wrong Lom transport')
    if 'lom_gpu_admission' in text or 'sophia_shell_work_stats' in text:
        raise ValueError('GPU admission or instrumentation present')
    if any(rows(text, tag) for tag in ('lom_cadence', 'lom_cadence_sample',
                                      'lom_cadence_final', 'lom_checksum_policy')):
        raise ValueError('cadence instrumentation in static capture')
    components = rows(text, 'sophia_shell_component')
    if components != [dict(schema='1', status='negotiated', slot='0', role='bar',
                           connection_epoch='1', content_grant_epoch='1', revision='10',
                           gpu_mode='denied', gpu_grant_epoch='0', device_major='0', device_minor='0')]:
        raise ValueError('expected one GPU-denied bar component')
    heads = rows(text, 'sophia_live_native_head')
    if len(heads) != 1:
        raise ValueError('expected one live native head')
    head = heads[0]
    if (head.keys() != {'schema','status','output','head','connector','connector_id','mode','refresh_millihz','mirrored'}
            or any(head.get(k) != v for k,v in dict(schema='2', status='ready', connector='DP-1',
                   mode='1920x1080', refresh_millihz='60000', mirrored='false').items())):
        raise ValueError('wrong native head mapping')
    for key in ['output', 'head', 'connector_id']:
        positive(head[key])
    candidates = rows(text, 'lom_panel_candidate')
    if len(candidates) != 1 or candidates[0].keys() != CANDIDATE:
        raise ValueError('expected exactly one Lom candidate')
    candidate = candidates[0]
    if (any(candidate[k] != v for k,v in dict(schema='1' if hashed else '2', status='presented',
            connection_epoch='1', content_grant_epoch='1', output=head['output'],
            candidate_generation='1', width='1920', height=str(height), bytes=str(1920*height*4),
            raster_source='rendered').items())
            or (re.fullmatch('[0-9a-f]{16}', candidate['checksum']) is None if hashed
                else candidate['checksum'] != 'unavailable')):
        raise ValueError('wrong opaque static candidate')
    positive(candidate['presentation_epoch'])
    positive(candidate['indicator_generation'])
    host = rows(text, 'sophia_live_shell_content')
    if len(host) != 3 or [row.get('status') for row in host] != ['outputs','prepared','presented']:
        raise ValueError('owner candidate changed or is not settled')
    if host[0] != dict(schema='1', status='outputs', facts_generation='1', outputs='1',
                       connection_epoch='1', content_grant_epoch='1'):
        raise ValueError('output facts changed')
    for row, fields in [(host[1], PREPARED), (host[2], PRESENTED)]:
        if row.keys() != fields or row['schema'] != '1':
            raise ValueError('unexpected owner record')
        if any(re.fullmatch('0|[1-9][0-9]*', v) is None or int(v) > 2**64-1
               for k,v in row.items() if k != 'status'):
            raise ValueError('noncanonical owner record')
        if row['output'] != candidate['output'] or row['candidate_generation'] != '1':
            raise ValueError('owner candidate mismatch')
        if (row['resident_bytes'] != candidate['bytes'] or row['backing_bytes'] != candidate['bytes']
                or row['staging_bytes'] != '0' or row['retiring_bytes'] != '0'):
            raise ValueError('owner allocation differs from static strip')
    if any(host[2][k] != candidate[k] for k in ['connection_epoch','content_grant_epoch','presentation_epoch']):
        raise ValueError('owner/client presentation mismatch')
    indicators = rows(text, 'sophia_shell_indicator_state')
    expected = [dict(schema='1', connection_epoch='1', indicator_generation=candidate['indicator_generation'],
                     output=head['output'], indicator=str(i+1), action=str(i+11), slot=str(i),
                     state_bits='1' if i == 0 else '0', entries='9') for i in range(9)]
    if indicators != expected:
        raise ValueError('admitted workspace state differs from static recipe')
    tag = 'sophia_native_composition_sampling'
    sampling = []
    for line in text.splitlines():
        if tag not in line:
            continue
        prefix, record = line.split(tag, 1)
        if re.fullmatch(r'\d{4}-\d\d-\d\dT[0-9:.]+Z  INFO sophia_renderer_native_egl::gl: ', prefix) is None:
            raise ValueError('unexpected sampling diagnostic prefix')
        sampling.extend(rows(tag + record, tag))
    common = dict(status='active', requested='exact_nearest', effective='exact_nearest',
                  alpha_mode='premultiplied', source=f'1920x{height}', target=f'1920x{height}')
    contract = dict(common, schema='3', output=head['output'], head=head['head'], frame='1920x1080')
    if not sampling:
        raise ValueError('missing exact-nearest sampling evidence')
    for row in sampling:
        if row != dict(contract, scene_generation=row.get('scene_generation')):
            raise ValueError('expected exact-nearest opaque 1:1 sampling path')
        positive(row['scene_generation'])
    # Draws can repeat while the one shell candidate stays unchanged.
    return dict(head=head, candidate=candidate, host=host, indicators=indicators, sampling=contract)
