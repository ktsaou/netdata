"""Reject lost tokens, reordered errors and incomplete computed fixture captures."""
from copy import deepcopy
import unittest

from capture import validate_frontend


def capture_fixture():
    number = {"type": 57361, "position": 0, "value": "MQ=="}
    eof = {"type": 57352, "position": 1, "value": ""}
    bad = {"type": 57353, "position": 0, "value": "ZXJyb3I="}
    error = {"start": 0, "end": 1, "line_offset": 0, "message": "ZXJyb3I=", "query_ref": "input"}
    ast = {"root": {"node_ref": 0}, "nodes": [{"node": "NumberLiteral", "fields": {"Val": {"float_bits": "3ff0000000000000"}}}]}
    return {"schema_version": 1, "original_assertions_passed": True,
            "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
            "token_catalog": {"57361": "NUMBER", "57352": "EOF", "57353": "ERROR"},
            "lexers": [{"id": f"lexer:00:{i:03d}", "input": "MQ==", "declared_failure": i < 52,
                        "series_description": i < 9, "tokens": [deepcopy(bad if i < 52 else number), deepcopy(eof)],
                        "declared_tokens": [deepcopy(number)]} for i in range(157)],
            "expressions": [{"id": f"parser:{i:03d}", "input": "MQ==", "declared_failure": i < 148,
                             "ast": deepcopy(ast), "declared_ast": deepcopy(ast),
                             "errors": [deepcopy(error)] if i < 148 else [],
                             "declared_errors": [deepcopy(error)] if i < 148 else []} for i in range(356)]}


class CaptureMutationTests(unittest.TestCase):
    def test_required_case_loss_and_duplicates_fail(self):
        data = capture_fixture()
        validate_frontend(data)
        data["expressions"].pop()
        with self.assertRaises(ValueError):
            validate_frontend(data)
        data = capture_fixture()
        data["lexers"][1]["id"] = data["lexers"][0]["id"]
        with self.assertRaises(ValueError):
            validate_frontend(data)

    def test_terminal_error_cannot_replace_eof(self):
        data = capture_fixture()
        data["lexers"][-1]["tokens"][-1]["type"] = 57353
        with self.assertRaises(ValueError):
            validate_frontend(data)

    def test_recoverable_error_is_preserved(self):
        data = capture_fixture()
        case = data["lexers"][-1]
        error = {"type": 57353, "position": 0, "value": "ZXJyb3I="}
        case["tokens"].insert(0, error)
        case["declared_tokens"].insert(0, deepcopy(error))
        validate_frontend(data)
        case["tokens"].pop(0)
        with self.assertRaises(ValueError):
            validate_frontend(data)

    def test_ordered_errors_and_raw_bits_cannot_change(self):
        data = capture_fixture()
        error = deepcopy(data["expressions"][0]["errors"][0])
        error["start"] = 1
        data["expressions"][0]["errors"].append(error)
        data["expressions"][0]["declared_errors"].append(deepcopy(error))
        validate_frontend(data)
        data["expressions"][0]["errors"].reverse()
        with self.assertRaises(ValueError):
            validate_frontend(data)
        data = capture_fixture()
        data["expressions"][-1]["ast"]["nodes"][0]["fields"]["Val"]["float_bits"] = "0000000000000000"
        with self.assertRaises(ValueError):
            validate_frontend(data)

    def test_invalid_bytes_and_query_references_fail(self):
        data = capture_fixture()
        data["lexers"][0]["input"] = "not base64!"
        with self.assertRaises(ValueError):
            validate_frontend(data)
        data = capture_fixture()
        data["expressions"][0]["errors"][0]["query_ref"] = "other"
        with self.assertRaises(ValueError):
            validate_frontend(data)


if __name__ == "__main__":
    unittest.main()
