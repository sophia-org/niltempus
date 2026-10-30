"""Readback gates for the single-output, untimed colour recipe."""
import json
import re


def niri(text):
    def unique(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate output key')
            result[key] = value
        return result
    outputs = json.loads(text, object_pairs_hook=unique)
    if type(outputs) is not dict or 'DP-1' not in outputs:
        raise ValueError('missing DP-1')
    for name, value in outputs.items():
        if type(value) is not dict or (name != 'DP-1' and value.get('current_mode') is not None):
            raise ValueError('foreign enabled output')
    out = outputs['DP-1']
    index = out['current_mode']
    if type(index) is not int or not 0 <= index < len(out['modes']):
        raise ValueError('invalid current mode')
    mode = out['modes'][index]
    if any(type(mode.get(k)) is not int or mode[k] != v for k, v in
           [('width', 1920), ('height', 1080), ('refresh_rate', 119880)]):
        raise ValueError('wrong native mode')
    logical = out['logical']
    if (out.get('vrr_enabled') is not False or type(logical) is not dict
            or logical != dict(x=0, y=0, width=1920, height=1080, scale=1, transform='Normal')
            or type(logical['scale']) not in (int, float)
            or any(type(logical[k]) is not int for k in ('x', 'y', 'width', 'height'))):
        raise ValueError('VRR, scale, position or transform')
    return dict(output='DP-1', width=1920, height=1080, refresh_millihz=119880,
                scale=1, transform='normal', vrr=False)


def xlibre(text):
    lines = text.splitlines()
    if not lines or not re.fullmatch(r'Screen 0: minimum \d+ x \d+, current 1920 x 1080, maximum \d+ x \d+', lines[0]):
        raise ValueError('X root geometry')
    headers = [(i, line) for i, line in enumerate(lines)
               if re.match(r'^\S+ (?:connected|disconnected)\b', line)]
    active = [(i, line) for i, line in headers if re.search(r'\b\d+x\d+\+\d+\+\d+', line)]
    if len(active) != 1 or not re.match(r'^DP-1 connected primary 1920x1080\+0\+0 \(0x[0-9a-f]+\) normal \(', active[0][1]):
        raise ValueError('X active output/rotation')
    start = active[0][0]
    end = next((i for i, _ in headers if i > start), len(lines))
    block = lines[start+1:end]
    transforms = [i for i, line in enumerate(block) if line.strip().startswith('Transform:')]
    if len(transforms) != 1:
        raise ValueError('X transform missing')
    i = transforms[0]
    matrix = ' '.join([block[i].split(':', 1)[1], *block[i+1:i+3]]).split()
    if matrix != ['1.000000', '0.000000', '0.000000', '0.000000', '1.000000', '0.000000', '0.000000', '0.000000', '1.000000']:
        raise ValueError('X transform not identity')
    modes = [i for i, line in enumerate(block) if '*current' in line]
    if len(modes) != 1 or not block[modes[0]].lstrip().startswith('1920x1080 '):
        raise ValueError('X current mode')
    timing = block[modes[0]+1:modes[0]+3]
    if len(timing) != 2 or not timing[0].strip().startswith('h:') or not re.search(r'\bclock 119\.88Hz$', timing[1]):
        raise ValueError('X refresh readback')
    return dict(output='DP-1', width=1920, height=1080, refresh_hz=119.88,
                scale=1, transform='normal',
                limits='Rounded xrandr refresh; VRR disabled by pinned X recipe, not proved by vrr_capable.')
