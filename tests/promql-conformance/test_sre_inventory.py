"""Exercise lexical boundaries and deterministic template expansion using synthetic sources."""
import unittest

from sre_inventory import comprehension_bindings, literal_list, pairs, render, scan, yaml_expressions


class SourceTests(unittest.TestCase):
    def test_comments_and_expression_strings_do_not_confuse_the_extractor(self):
        tokens = scan('''locals {
          // expr = "ignored"
          /* expr = "also ignored" */
          expr = "absent(m{label=\\"# not a comment\\"})" # ignored
        }''')
        values = [t.value for t in tokens if t.kind == "string"]
        self.assertEqual(values, ['absent(m{label="# not a comment"})'])
        self.assertTrue(pairs(tokens))

    def test_indented_heredoc_preserves_contents_and_newline(self):
        tokens = scan('''expr = <<-END
          sum(m)
        END
''')
        self.assertEqual(tokens[-1].value, "  sum(m)\n")

    def test_unclosed_and_nonliteral_inputs_fail(self):
        for source in ['expr = "unfinished', '/* unfinished', 'expr = <<END\nunclosed']:
            with self.assertRaises(ValueError):
                scan(source)
        tokens = scan('["a", local.dynamic]')
        with self.assertRaises(ValueError):
            literal_list(tokens, 0, pairs(tokens))

    def test_comprehensions_include_two_variable_maps(self):
        header = scan('for severity, threshold in { "warning" = 14, "critical" = 7 }')
        values = comprehension_bindings(header, "test", [], [])
        self.assertEqual(values, [{"severity": "critical", "threshold": "7"},
                                  {"severity": "warning", "threshold": "14"}])
        header = scan('for service in concat(["fixed"], local.microservices)')
        self.assertEqual(comprehension_bindings(header, "test", ["first", "second"], []),
                         [{"service": "fixed"}, {"service": "first"}, {"service": "second"}])

    def test_nested_bindings_and_unknown_directives(self):
        url = render('https://${local.domain}', {"local.domain": "example.invalid"})
        self.assertEqual(render("absent(m{url='${item.url}'})", {"item.url": url}),
                         "absent(m{url='https://example.invalid'})")
        for query in ['m > ${unknown}', 'm %{ if unknown }']:
            with self.assertRaises(ValueError):
                render(query, {})

    def test_recording_outputs_and_dependency_source_are_preserved(self):
        source = """groups:
- name: test
  rules:
  - record: test:rate:5m
    expr: rate(test_counter[5m])
    labels:
      scope: test
  - alert: HighRate
    expr: test:rate:5m > 1
"""
        expressions = yaml_expressions(source, "rules.yaml")
        self.assertEqual(len(expressions), 2)
        self.assertEqual(expressions[0]["rule_name"], "test:rate:5m")
        self.assertEqual(expressions[0]["labels"], {"scope": "test"})
        self.assertEqual(expressions[1]["query"], "test:rate:5m > 1")
        self.assertEqual([e["group_index"] for e in expressions], [0, 1])


if __name__ == "__main__":
    unittest.main()
