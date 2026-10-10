"""Native CI-only Japanese baseline/restoration probe. No models or downloads."""
import argparse
import inspect
import json
from pathlib import Path
import sys

# LUMA_NAGISA_COMPAT_SOURCE


def restored_state(cwd, finders, adapted):
    # Ordinary Nagisa dependencies may install their own import finders (six).
    # Preserve those upstream effects, then compare them with the genuine ASCII
    # baseline. The application's transient finder must always be absent.
    canonical_cwd = Path.cwd().resolve() == cwd.resolve()
    retained = tuple(item for item in sys.meta_path if any(item is before for before in finders))
    additions = [type(item).__module__ + '.' + type(item).__qualname__ for item in sys.meta_path
                 if all(item is not before for before in finders)]
    luma_hook_absent = not any('luma_prepare_nagisa.<locals>.Finder' in type(item).__qualname__ for item in sys.meta_path)
    diagnostic = {'canonical_cwd_equal': canonical_cwd, 'original_finders_retained': retained == finders,
                  'cwd_before': str(cwd), 'cwd_after': str(Path.cwd()),
                  'finders_before': [{'type': type(item).__module__ + '.' + type(item).__qualname__, 'identity': id(item)} for item in finders],
                  'finders_after': [{'type': type(item).__module__ + '.' + type(item).__qualname__, 'identity': id(item)} for item in sys.meta_path],
                  'luma_hook_absent': luma_hook_absent, 'upstream_finders_added': additions}
    print('NAGISA_IMPORT_STATE=' + json.dumps(diagnostic), file=sys.stderr, flush=True)
    assert canonical_cwd and retained == finders and luma_hook_absent
    if adapted:
        assert _luma_nagisa_restoration == {'constructor_restored': True, 'finder_removed': True, 'cwd_restored': True, 'original_finders_retained': True}
    return diagnostic


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--adapt', action='store_true')
    parser.add_argument('--forced-failure', action='store_true'); args = parser.parse_args()
    assert sys.platform == 'win32' and sys.flags.isolated
    def audit(event, _args):
        if event in {'socket.connect', 'socket.getaddrinfo', 'socket.bind', 'subprocess.Popen', 'os.system', 'os.posix_spawn', 'os.fork'}:
            raise RuntimeError('Nagisa probe is offline and cannot start child processes')
    sys.addaudithook(audit)
    cwd = Path.cwd(); finders = tuple(sys.meta_path)
    _luma_validate_nagisa()
    if args.forced_failure:
        original = _luma_importlib.import_module
        def fail_after_import(name, *args, **kwargs):
            result = original(name, *args, **kwargs)
            if name == 'nagisa': raise RuntimeError('Deliberate post-import restoration test')
            return result
        _luma_importlib.import_module = fail_after_import
        try:
            luma_prepare_nagisa()
            raise AssertionError('Expected fatal initialization error')
        except LumaNagisaInitializationError:
            assert _luma_nagisa_state == 'failed'
        finally:
            _luma_importlib.import_module = original
        restored = restored_state(cwd, finders, True)
        tagger = sys.modules['nagisa.tagger'].Tagger
        assert str(inspect.signature(tagger.__init__)) == '(self, vocabs=None, params=None, hp=None, single_word_list=None)'
        print(json.dumps({'fatal_failure_tested': True, 'cwd_restored': True, 'hooks_restored': True, 'process_discarded': True, **restored}))
        return  # This isolated probe process exits without accepting more work.
    if args.adapt:
        module = luma_prepare_nagisa()
    else:
        assert str(Path(sys.prefix)).isascii(), 'The genuine baseline requires an ASCII runtime path'
        module = _luma_importlib.import_module('nagisa')
    restored = restored_state(cwd, finders, args.adapt)
    assert module.Tagger.__init__ is sys.modules['nagisa.tagger'].Tagger.__init__
    if args.adapt:
        assert module.Tagger.__init__ is _luma_nagisa_original_init
    assert Path(module.Tagger.__init__.__code__.co_filename).resolve() == (Path(sys.prefix) / 'Lib/site-packages/nagisa/tagger.py').resolve()
    assert str(inspect.signature(module.Tagger.__init__)) == '(self, vocabs=None, params=None, hp=None, single_word_list=None)'
    text = 'Pythonで簡単に使えるツールです'
    tagged = module.tagging(text)
    assert module.wakati(text) == tagged.words
    assert tagged.words == ['Python', 'で', '簡単', 'に', '使える', 'ツール', 'です']
    print(json.dumps({'words': tagged.words, 'postags': tagged.postags, 'adapted': args.adapt,
                      'cwd_restored': True, 'hooks_restored': True, 'record_verified': True, **restored}, ensure_ascii=False))


if __name__ == '__main__': main()
