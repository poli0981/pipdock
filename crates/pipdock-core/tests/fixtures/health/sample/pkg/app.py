import os
import json
import requests


class Handler:
    def handle(self):
        return requests.get("https://example.com")

    def dead(self):
        return 1
        print("unreachable")


def unused_helper():
    pass


CONFIG = {"a": 1}
