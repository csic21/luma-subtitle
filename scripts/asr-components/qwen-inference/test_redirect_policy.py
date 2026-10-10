"""Offline exact-CDN redirect and bounded hostname diagnostic regressions."""
import copy
from email.message import Message
import hashlib
import io
import json
import sys
from pathlib import Path
import unittest
from unittest.mock import patch
import urllib.response

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
from test_download_diagnostics import m, MODELS, AUDIO, ENTRIES, request_file, error_line

HOSTS = ('us.aws.cdn.hf.co', 'us.gcp.cdn.hf.co')
SECRET = 'never-emit-signed-query-or-userinfo'


class RedirectPolicy(unittest.TestCase):
    def test_only_exact_documented_cdns_are_added_for_models(self):
        for host in HOSTS:
            for target in (f'https://{host}/object', f'https://{host.upper()}/object',
                           f'https://{host}:443/object?Signature={SECRET}'):
                with self.subTest(target=target.split('?')[0]):
                    self.assertTrue(m.allowed_download_url(target, True))
                    self.assertFalse(m.allowed_download_url(target, False))
            for target in (f'http://{host}/object', f'https://{host}:444/object',
                           f'https://user:password@{host}/object', f'https://user@{host}/object',
                           f'https://:password@{host}/object', f'https://@{host}/object',
                           f'https://:@{host}/object', f'https://{host}:invalid/object',
                           f'https://{host}:65536/object', f'https://{host}/object#fragment',
                           f'https://{host}.attacker.example/object', f'https://evil-{host}/object',
                           f'https://{host}./object'):
                with self.subTest(rejected=target):
                    self.assertFalse(m.allowed_download_url(target, True))
        for target in ('https://hf.co/object', 'https://cdn.hf.co/object',
                       'https://other.cdn.hf.co/object', 'https://s3.amazonaws.com/object',
                       'https://storage.googleapis.com/object', 'https://[malformed/object',
                       'https:///missing-host', ''):
            self.assertFalse(m.allowed_download_url(target, True))

    def opener(self, target, payload, requests):
        class Transport(m.urllib.request.HTTPSHandler):
            def https_open(self, request):
                requests.append(request)
                headers = Message()
                if len(requests) == 1:
                    headers['Location'] = target
                    body = io.BytesIO(b''); status = 302
                else:
                    headers['Content-Length'] = str(len(payload))
                    body = io.BytesIO(payload); status = 200
                response = urllib.response.addinfourl(body, headers, request.full_url, status)
                response.msg = 'offline fixture'
                return response
        real_build = m.urllib.request.build_opener
        return lambda *handlers: real_build(*handlers, Transport())

    def test_actual_cdn_redirects_still_require_exact_payload_size_and_hash(self):
        payload = b'pinned fixture bytes'
        for host in HOSTS:
            for size, digest, category in ((len(payload), hashlib.sha256(payload).hexdigest(), None),
                    (len(payload)-1, hashlib.sha256(payload).hexdigest(), 'size-mismatch'),
                    (len(payload)+1, hashlib.sha256(payload).hexdigest(), 'size-mismatch'),
                    (len(payload), 'f'*64, 'hash-mismatch')):
                with self.subTest(host=host,category=category,size=size),temporary_root() as root:
                    requests=[];target=f'https://{host}/object?Signature={SECRET}'
                    item={'url':ENTRIES[0][0]['url'],'bytes':size,'sha256':digest}
                    with patch.object(m.urllib.request,'build_opener',side_effect=self.opener(target,payload,requests)):
                        if category:
                            with self.assertRaises(m.FixtureDownloadError) as caught:
                                m.download(item,root/'payload',m.time.monotonic()+30)
                            self.assertEqual(caught.exception.evidence['category'],category)
                            self.assertNotIn(SECRET,json.dumps(caught.exception.evidence))
                        else:
                            m.download(item,root/'payload',m.time.monotonic()+30)
                            self.assertEqual((root/'payload').read_bytes(),payload)
                    self.assertEqual([request.full_url for request in requests],[item['url'],target])

    def test_redirect_depth_remains_bounded_with_new_cdn_hosts(self):
        requests=[]
        class Transport(m.urllib.request.HTTPSHandler):
            def https_open(self,request):
                requests.append(request);headers=Message()
                headers['Location']=f'https://{HOSTS[len(requests)%2]}/hop-{len(requests)}'
                response=urllib.response.addinfourl(io.BytesIO(b''),headers,request.full_url,302)
                response.msg='offline fixture';return response
        opener=m.urllib.request.build_opener(m.urllib.request.ProxyHandler({}),m.FixtureRedirects(True),Transport())
        with self.assertRaises(m.urllib.error.HTTPError):
            opener.open(ENTRIES[0][0]['url'],timeout=30)
        self.assertEqual(m.FixtureRedirects.max_redirections,8)
        self.assertEqual(len(requests),9)

    def test_rejected_host_roundtrips_without_url_credentials_path_or_query(self):
        target=f'https://user:{SECRET}@Unreviewed.Example/{SECRET}?Signature={SECRET}#{SECRET}'
        for readme in (False,True):
            with self.subTest(readme=readme),temporary_root() as root:
                request=request_file(root);requests=[]
                with patch.object(m.urllib.request,'build_opener',side_effect=self.opener(target,b'',requests)):
                    with self.assertRaises(m.FixtureDownloadError) as caught:
                        if readme:m.download_readme(m.time.monotonic()+30)
                        else:m.fetch_pair(MODELS,AUDIO,root)
                record=caught.exception.evidence
                self.assertEqual(record['category'],'redirect-rejected')
                self.assertEqual(record['redirect_host'],'unreviewed.example')
                self.assertEqual(record['bytes_received'],0)
                self.assertEqual(len(requests),1)
                serialized=error_line(record)
                self.assertLessEqual(len(serialized.encode('utf-8')),m.MAX_DOWNLOAD_ERROR_BYTES)
                for forbidden in (SECRET,'https://','Signature','user:'):
                    self.assertNotIn(forbidden,serialized)
                validate=m.readme_child_error if readme else lambda text:m.child_error(text,request)
                self.assertEqual(validate(serialized),record)
                malformed=[]
                for host in ('',None,False,'A.example','a'*254,'a'*64+'.example',
                             'https://evil.example','evil.example/path','evil.example?token=x',
                             'user@evil.example','evil.example:443','é.example','evil.example\n'):
                    bad=copy.deepcopy(record);bad['redirect_host']=host;malformed.append(bad)
                bad=copy.deepcopy(record);bad['category']='http-error';malformed.append(bad)
                bad=copy.deepcopy(record);bad['redirect_url']=target;malformed.append(bad)
                for bad in malformed:
                    with self.assertRaises(ValueError):validate(error_line(bad))
                longest='.'.join(['a'*63]*3+['b'*61])
                good=copy.deepcopy(record);good['redirect_host']=longest
                self.assertEqual(len(longest),253)
                self.assertEqual(validate(error_line(good)),good)

    def test_hostname_extraction_never_serializes_arbitrary_url_text(self):
        for target in ('https://[malformed', 'https://a_b.example/object',
                       'https://'+'a'*254+'/object', 'https://é.example/object',
                       'https://evil%2fexample/object', 'not-a-url'):
            self.assertIsNone(m.redirect_diagnostic_hostname(target))
        error=m.DownloadCheckError('redirect-rejected',f'https://evil.example?Signature={SECRET}')
        self.assertEqual(m.redirect_error_details(error),{})
        self.assertEqual(str(error),'redirect-rejected')
        self.assertEqual(m.redirect_error_details(m.DownloadCheckError('http-status','unreviewed.example')),{})


if __name__=='__main__':unittest.main()
