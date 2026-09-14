from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer
from pathlib import Path
from backend.apps.icons.icon_search import icons_obj
from database.service import add_data, delete_data

class MyWatcher(FileSystemEventHandler):
    def on_created(self, event):
        path = event.src_path
        if Path(path).suffix == ".desktop":
            file_name = Path(path).name
            icon_path = icons_obj.get_icon(path)
            add_data(file_name.split(".desktop")[0], path, icon_path)
    def on_deleted(self, event):
        path = event.src_path
        if Path(path).suffix == ".desktop":
           
            filename = Path(path).name
            delete_data(filename.split(".desktop")[0])

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
    