from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer
from pathlib import Path
from backend.apps.icons.icon_search import icons_obj
from database.service import add_data, delete_data
from backend.config_parser.async_config_parser import AsyncConfigParser
import asyncio

class MyWatcher(FileSystemEventHandler):
    def on_created(self, event):
        path = event.src_path
        if Path(path).suffix == ".desktop":
            desktop = asyncio.run(AsyncConfigParser.from_file(path))
            icon_path = icons_obj.get_icon(path)
            asyncio.run(add_data(desktop.get_app_name(), desktop.get_categories(), path, icon_path, desktop.get_command()))
    def on_deleted(self, event):
        path = event.src_path
        if Path(path).suffix == ".desktop":
           
            asyncio.run(delete_data(path))

def start_watcher():
    path = "/usr/share/applications"
    event_handler = MyWatcher()
    observer = Observer()
    observer.schedule(event_handler, path, recursive=True)
    observer.start()
    return observer

if __name__ == "__main__":
    observer = start_watcher()
    try:
        while True:
            observer.join(1)
    finally:
        observer.stop()
        observer.join()
