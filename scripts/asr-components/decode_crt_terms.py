"""Decode the two exact CRT RTFs, honoring font encodings; no network."""
import argparse
import hashlib
from pathlib import Path
import re
import subprocess

PINS={'en': '8099dc3cf9502c335da829e5c755948a12e3e6de490eb492a99deb673d883d8b',
      'zh-CN': 'a808b4933ce3b3e0893504dbef43ebf90b8b567f94bd6481b6315ed9141e1b11'}
IGNORED={'ansi','ansicpg','b','blue','cf','deff','deflang','fcharset','fi','fnil','fs','green','lang','li','nouicompat','pard','pnf','pnindent','pnlvlblt','pntxtb','red','rtf','sa','sb','sl','slmult','tx','uc','ul','ulnone','viewkind','field','fldrslt','pntext'}
DESTINATIONS={'fonttbl','colortbl','generator','pn','fldinst'}
TOKEN=re.compile(r"(\{)|(\})|((?:\\'[0-9a-fA-F]{2})+)|\\([A-Za-z]+)(-?\d+)? ?|\\([^A-Za-z])|([^{}\\]+)")


def decode(raw):
    text=raw.decode('ascii')
    fonts={int(n): int(charset) for n,charset in re.findall(r'\{\\f(\d+)\\fnil\\fcharset(\d+) ',text)}
    assert fonts[0]==0 and fonts[2]==2 and fonts[1] in (0,134)
    state={'font':0,'skip':False}; stack=[]; plain=[]; normalized=[]; end=0
    for match in TOKEN.finditer(text):
        assert match.start()==end; end=match.end()
        opening,closing,hexes,word,arg,symbol,literal=match.groups()
        original=match.group(); replacement=original
        if opening: stack.append(state.copy())
        elif closing: state=stack.pop()
        elif hexes:
            data=bytes.fromhex(hexes.replace("\\'",''))
            charset=fonts[state['font']]
            decoded=data.decode('gbk' if charset==134 else 'cp1252',errors='strict')
            if not state['skip']: plain.append(decoded)
            assert all(ord(c)<=65535 for c in decoded)
            replacement=''.join('\\u'+str(ord(c) if ord(c)<32768 else ord(c)-65536)+'?' for c in decoded)
        elif word:
            if word=='f': state['font']=int(arg)
            elif word in DESTINATIONS: state['skip']=True
            elif word=='par':
                if not state['skip']: plain.append('\n\n')
            elif word=='tab':
                if not state['skip']: plain.append(' ')
            elif word in ('ldblquote','rdblquote'):
                if not state['skip']: plain.append('“' if word=='ldblquote' else '”')
            elif word not in IGNORED: raise ValueError('Unreviewed RTF control '+word)
        elif symbol:
            if symbol=='*': state['skip']=True
            elif symbol in ('{','}','\\'):
                if not state['skip']: plain.append(symbol)
            else: raise ValueError('Unreviewed RTF symbol '+symbol)
        elif literal and not state['skip']:
            plain.append(literal.replace('\r','').replace('\n','').replace('\x00',''))
        normalized.append(replacement)
    assert end==len(text) and not stack
    return ''.join(plain),''.join(normalized)


def compact(text):
    # Paragraph/indentation normalization only. Every non-whitespace character
    # must agree between the independent direct decoder and Pandoc's RTF reader.
    return ''.join(c for c in text if not c.isspace())


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-dir', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args=parser.parse_args();root=args.source_dir.resolve();output=args.output_dir.resolve();output.mkdir(parents=True,exist_ok=True)
    assert subprocess.check_output(['pandoc','--version'],text=True).splitlines()[0]=='pandoc 3.1.11.1'
    for locale,pin in PINS.items():
        raw=(root/f'license-{locale}.rtf').read_bytes();assert hashlib.sha256(raw).hexdigest()==pin
        direct,normalized=decode(raw)
        normalized_path=output/f'license-{locale}.unicode-normalized.rtf';normalized_path.write_text(normalized,encoding='ascii')
        result=subprocess.check_output(['pandoc','-f','rtf','-t','plain','--wrap=none',str(normalized_path)],text=True)
        assert compact(direct)==compact(result),locale+' independent conversion differs'
        assert '\ufffd' not in result and '\x00' not in result
        if locale=='zh-CN':assert '软件许可条款' in result and '您可以安装和使用任意数量的软件副本' in result
        path=output/f'license-{locale}.verified.txt';path.write_text(result,encoding='utf-8',newline='\n')
        print(locale,len(path.read_bytes()),hashlib.sha256(path.read_bytes()).hexdigest(),'all non-whitespace characters independently matched')
