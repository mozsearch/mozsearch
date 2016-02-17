from locust import HttpLocust, TaskSet, task
from bs4 import BeautifulSoup
import random
import re
import urllib
import json

BASE = "/mozilla-central/source"

class UserBehavior(TaskSet):
    def __init__(self, l):
        super(UserBehavior, self).__init__(l)

        self.queue = [BASE]
        self.dynamic = set()

        self.latest_document = ''
        self.latest_path = '/'

    @task(1)
    def help_page(self):
        self.client.get("/")

    @task(20)
    def found_link(self):
        if random.choice([True, False]) and len(self.dynamic):
            url = random.choice(list(self.dynamic))
        else:
            url = random.choice(self.queue + [BASE])

        response = self.client.get(url)

        self.latest_document = response.text
        self.latest_path = url[len(BASE):]

        new_level = []

        for m in re.finditer(r'a href="([^"]*)"', response.text):
            url = m.group(1)
            if url.startswith('/'):
                new_level.append(url)

        #soup = BeautifulSoup(response.text, "html.parser")
        #for res in soup.find_all(href=True):
        #    url = res['href']
        #    if url.startswith('/'):
        #        new_level.append(url)

        analysis_data = re.search(r'var ANALYSIS_DATA = (.*);', response.text)
        if analysis_data:
            analysis_data = json.loads(analysis_data.group(1))
            for datum in analysis_data:
                [jumps, searches] = datum
                for j in jumps:
                    self.dynamic.add('/mozilla-central/define?q=' + urllib.quote(j['sym']))
                for s in searches:
                    self.dynamic.add('/mozilla-central/search?q=symbol:' + urllib.quote(s['sym']))

        self.queue = new_level

    def select(self, s):
        if len(s) <= 1:
            return s

        length = random.randint(1, min(20, len(s)))
        begin = random.randint(0, len(s) - length)
        return s[begin:begin+length]

    def add_chars(self, s, chars):
        count = random.randint(0, 4)
        for i in range(count):
            pos = random.randint(0, len(s))
            which = random.randint(0, len(chars) - 1)
            s = s[:pos] + chars[which] + s[pos:]
        return s

    @task(20)
    def full_text_search(self):
        text = self.select(self.latest_document)

        # re?
        if random.choice([True, False]):
            text = 're:' + self.add_chars(text, ".?*()|^$\\[]")

        # path?
        if random.choice([True, False]):
            path = self.select(self.latest_path)

            # pathre?
            if random.choice([True, False]):
                path = self.add_chars(path, ".?*()|^$\\[]")
                text = 'pathre:' + path + ' ' + text
            else:
                text = 'path:' + path + ' ' + text
        
        print text
        response = self.client.get('/mozilla-central/search?q=' + urllib.quote(text))

class WebsiteUser(HttpLocust):
    task_set = UserBehavior
    min_wait=500
    max_wait=1000

