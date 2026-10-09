"""Small cached Tiny/JFK fixtures for the existing real Rust worker test."""
import json
from pathlib import Path
import shutil
import wave
from build import ROOT, dump, safe_name, sha256

CPU_FILENAME = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
CPU_URL = 'https://github.com/csic21/luma-subtitle/releases/download/asr-ct2-cpu-4.8.2-1/' + CPU_FILENAME
REPEATS = 100
MAX_REPEAT_BYTES = 36 * 1024 * 1024


def final_cpu_recipe(candidate):
    recipe = candidate.get('recipe')
    if candidate.get('id') != 'faster-whisper-cpu-windows-x64' or not recipe:
        return False
    matches = [w for w in recipe['wheels'] if w['name'] == 'ctranslate2']
    if len(matches) != 1: return False
    wheel = matches[0]
    if (wheel['filename'], wheel['url'], wheel['version']) != (CPU_FILENAME, CPU_URL, '4.8.2'):
        return False
    pinned = next(w for w in json.loads((ROOT/'locks/faster-whisper-cpu-windows-x64.json').read_text(encoding='utf-8'))['wheels'] if w['name'] == 'ctranslate2')
    if any(wheel[field] != pinned[field] for field in ('filename','url','version','sha256','bytes')):
        raise ValueError('CPU worker proof recipe differs from the reviewed source lock')
    if recipe.get('windows_crt') != 'msvc-14.44.35211-x64':
        raise ValueError('Final CPU worker proof requires its fixed private CRT')
    return True


def copy_cached(item, cache, destination):
    source = cache/item['sha256']
    if source.is_symlink() or not source.is_file() or source.stat().st_size != item['bytes'] or sha256(source) != item['sha256']:
        raise ValueError('Real worker fixture must already exist in the verified Tiny/JFK cache')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)


def repeat_audio(source, destination):
    with wave.open(str(source),'rb') as original:
        if (original.getnchannels(),original.getsampwidth(),original.getframerate(),original.getcomptype()) != (1,2,16000,'NONE'):
            raise ValueError('Cancellation fixture requires mono16kHz PCM16')
        frames = original.getnframes()
        if frames != 176000: raise ValueError('Cancellation fixture requires the reviewed eleven-second JFK clip')
        pcm = original.readframes(frames)
        if len(pcm) != frames*2: raise ValueError('Truncated source audio')
    if len(pcm)*REPEATS+44 > MAX_REPEAT_BYTES: raise ValueError('Cancellation fixture exceeds its fixed bound')
    with wave.open(str(destination),'wb') as output:
        output.setnchannels(1);output.setsampwidth(2);output.setframerate(16000)
        for _ in range(REPEATS): output.writeframesraw(pcm)
    return {'repeats':REPEATS,'bytes':destination.stat().st_size,'sha256':sha256(destination),
            'duration_ms':frames*REPEATS*1000//16000}


def prepare(cache, directory):
    directory.mkdir(parents=True,exist_ok=False)
    pins=json.loads((ROOT/'fixtures.json').read_text(encoding='utf-8'))
    model=directory/'tiny model';audio=directory/'jfk.wav';long_audio=directory/'jfk-repeat.wav'
    for item in pins['faster_whisper_tiny']['files']:
        copy_cached(item,cache,model.joinpath(*safe_name(item['path']).parts))
    copy_cached(pins['audio'],cache,audio)
    repeated=repeat_audio(audio,long_audio)
    evidence={'model_revision':pins['faster_whisper_tiny']['version'],'audio_sha256':pins['audio']['sha256'],
              'cancellation_audio':repeated,'new_downloads':False}
    dump(directory/'fixture.json',evidence)
    return {'LUMA_ASR_TEST_MODEL':str(model),'LUMA_ASR_TEST_AUDIO':str(audio),
            'LUMA_ASR_TEST_LONG_AUDIO':str(long_audio),'LUMA_ASR_TEST_OUTPUT':str(directory/'results')},evidence
