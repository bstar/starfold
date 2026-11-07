"""Guard against inflated compatibility claims from the evidence parser."""
import json
from pathlib import Path
import tempfile
import unittest
import base64
from types import SimpleNamespace
from unittest.mock import patch

from hypothesis import given, settings, strategies as st

from PIL import Image
from lab import Case, collect_evidence, help_open, image_protocol, protocol_observations, quadrant_pattern, report, sgr_colors


class EvidenceTests(unittest.TestCase):
    def test_graphical_assertions_read_pixels_and_cache_unchanged_frames(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def screenshot(path):
                image = Image.new('RGB', (100, 100), 'black')
                image.paste('#202020', (20, 20, 80, 80))
                image.save(path)
                return image
            case = Case.__new__(Case)
            case.profile, case.native, case.bridge = 'graphical', True, root
            case.ocr_cache = None
            case.display = SimpleNamespace(screenshot=screenshot)
            with patch('lab.command', return_value=SimpleNamespace(stdout=b'10-source. txt\nKEYS\nnew tab')) as ocr:
                with patch('lab.observe', side_effect=AssertionError('Cell text must not supply graphical assertions')):
                    self.assertIn('10-source.txt', case.screen())
                    # The alternate reading doubles the cropped 60x60 window.
                    with Image.open(root / 'ocr.png') as captured:
                        self.assertEqual(captured.size, (120, 120))
                    self.assertTrue(help_open(case.screen()))
                    self.assertEqual(ocr.call_count, 2)
            self.assertTrue((root / 'ocr.txt').is_file())

    def test_report_separates_graphical_assertions_from_historical_auto(self):
        cases = [
            {'terminal': 'sample', 'profile': 'auto', 'presentation': 'native',
             'checks': {'copy-operations': {'status': 'pass', 'detail': 'Historical cells'}}},
            {'terminal': 'sample', 'profile': 'graphical', 'presentation': 'native',
             'assertion_presentation': 'graphical',
             'checks': {'copy-operations': {'status': 'pass', 'detail': 'Real graphical copy'}}},
            {'terminal': 'fallback', 'profile': 'graphical', 'presentation': 'cells',
             'assertion_presentation': 'unavailable',
             'checks': {'graphical-mode': {'status': 'unsupported', 'detail': 'Cell fallback'}}},
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report(root, cases, {})
            import csv
            with (root / 'matrix.csv').open() as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual([row['Functional checks in'] for row in rows],
                             ['cells', 'graphical', 'unavailable'])
            self.assertEqual(rows[2]['graphical-mode'], 'unsupported')
            self.assertEqual(rows[2]['copy-operations'], 'not-run')

    def test_protocol_columns_use_wire_evidence_and_keep_unknowns(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            case = root / 'sample-auto'
            case.mkdir()
            (case / 'input.bin').write_bytes(b'\x1b[<65;10;20M\x1b[97;1u')
            result = {'terminal': 'sample', 'profile': 'auto', 'evidence': case.name,
                      'image_protocol': 'sixel', 'checks': {
                          'desktop-dnd': {'status': 'manual', 'detail': 'Needs desktop interaction'},
                          'native-ui': {'status': 'unsupported', 'detail': 'Cell fallback'}}}
            protocols = protocol_observations(root, result)
            self.assertEqual(protocols['Sixel'], 'observed')
            self.assertEqual(protocols['Kitty graphics'], 'not-observed')
            self.assertEqual(protocols['SGR mouse'], 'observed')
            self.assertEqual(protocols['Kitty keyboard'], 'observed')
            self.assertEqual(protocols['Bracketed paste'], 'not-observed')
            self.assertEqual(protocols['OSC 72 drag/drop'], 'manual')
            report(root, [result], {})
            for name in ('index.html', 'matrix.csv', 'REPORT.md'):
                content = (root / name).read_text()
                self.assertIn('Drag and drop', content)
                self.assertIn('OSC 52 clipboard', content)
                self.assertIn('Kitty graphics', content)
            self.assertIn('.fail,.unsupported{color:#ff8b91}', (root / 'index.html').read_text())

    def test_empty_report_has_one_filter_and_no_claimed_passes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report(root, [], {})
            page = (root / 'index.html').read_text()
            self.assertEqual(page.count('id="terminal-filter"'), 1)
            self.assertIn('0 cases', page)

    def test_collection_recovers_newer_case_result_without_losing_unavailable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            case = root / 'kitty-auto'
            case.mkdir()
            (case / 'result.json').write_text(json.dumps({'terminal': 'kitty', 'profile': 'auto', 'checks': {'startup': {'status': 'pass'}}}))
            saved = {'metadata': {}, 'results': [{'terminal': 'kitty', 'profile': 'auto', 'checks': {'startup': {'status': 'fail'}}}, {'terminal': 'warp', 'status': 'unavailable'}]}
            result = collect_evidence(root, saved, {'terminals': [{'id': 'kitty'}, {'id': 'warp'}]})
            self.assertEqual(len(result['results']), 2)
            self.assertEqual(result['results'][0]['checks']['startup']['status'], 'pass')
            self.assertEqual(result['results'][1]['status'], 'unavailable')

    @settings(max_examples=100, deadline=None)
    @given(st.binary(max_size=4096))
    def test_arbitrary_query_payloads_are_not_image_evidence(self, payload):
        query = b'\x1b_Ga=q,i=31,s=1,v=1;' + base64.b64encode(payload) + b'\x1b\\'
        self.assertEqual(image_protocol(query), 'none-observed')

    @settings(max_examples=100, deadline=None)
    @given(st.lists(st.integers(min_value=0, max_value=65535), min_size=3, max_size=3))
    def test_only_valid_rgb_channels_are_counted(self, channels):
        sequence = b'\x1b[38;2;' + b';'.join(str(c).encode() for c in channels) + b'm'
        colors = sgr_colors(sequence)
        if all(c < 256 for c in channels):
            self.assertEqual(colors, ['#%02x%02x%02x' % tuple(channels)])
        else:
            self.assertEqual(colors, [])

    def test_help_does_not_require_offscreen_categories(self):
        self.assertTrue(help_open('KEYS\nctrl+t new tab\nnavigation'))
        self.assertFalse(help_open('ordinary file listing'))

    def test_graphics_query_does_not_count_as_an_image(self):
        self.assertEqual(image_protocol(b'\x1b_Ga=q,i=31,s=1,v=1;AAAA\x1b\\'), 'none-observed')
        self.assertEqual(image_protocol(b'\x1b_Ga=q,i=31;A\x1b\\' + '▀'.encode()), 'half-blocks')
        self.assertEqual(image_protocol(b'\x1b_Ga=T,f=32,s=4,v=4;DATA\x1b\\'), 'kitty')
        self.assertEqual(image_protocol(b'\x1b_Gf=100,a=t;DATA\x1b\\'), 'kitty')
        self.assertEqual(image_protocol(b'\x1bP0;1qimage\x1b\\'), 'sixel')
        self.assertEqual(image_protocol(b'\x1b]1337;File=inline=1:DATA\x07'), 'iterm2')

    def test_colors_reject_out_of_range_channels(self):
        self.assertEqual(sgr_colors(b'\x1b[38;2;10;20;30m\x1b[48;2;999;1;2m'), ['#0a141e'])

    def test_palette_reduction_preserves_fixture_geometry(self):
        image = Image.new('RGB', (240, 180), '#202020')
        colors = ['red', 'cyan', 'blue', 'yellow']
        for index, color in enumerate(colors):
            x, y = 40 + index % 2 * 60, 24 + index // 2 * 48
            image.paste(color, (x, y, x + 60, y + 48))
        self.assertIsNotNone(quadrant_pattern(image))
        self.assertIsNone(quadrant_pattern(Image.new('RGB', (240, 180), 'blue')))
        image.paste('red', (40, 24, 160, 120))
        self.assertIsNone(quadrant_pattern(image))

    def test_report_keeps_unavailable_and_unverified_results_visible(self):
        result = {'terminal': '<fake>', 'version': '1', 'status': 'unavailable',
                  'detail': '<script>no</script>', 'checks': {'audio': {'status': 'not-run', 'detail': 'No device test'}}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report(root, [result], {'software_rendering': True})
            page = (root / 'index.html').read_text()
            self.assertIn('&lt;script&gt;', page)
            self.assertNotIn('<script>no</script>', page)
            stored = json.loads((root / 'results.json').read_text())
            self.assertEqual(stored['results'][0]['checks']['audio']['status'], 'not-run')
            self.assertIn('unavailable', (root / 'REPORT.md').read_text())


if __name__ == '__main__':
    unittest.main()
