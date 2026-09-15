import signal
import sys
from pathlib import Path

from PySide6 import QtCore, QtWidgets

from backend.apps.apps_tracker.tracker import start_watcher
from backend.apps.application_search import apps_obj

from database.init import init_db

import asyncio

# Сохраняет запуск как `python frontend/app.py` и одновременно позволяет
# backend импортировать `frontend.app` как обычный пакет.
PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from frontend.window import MainWindow


def main(applications=None, badges=None):
    asyncio.run(init_db())
    if not apps_obj.get_apps().apps:
        apps_obj.initial_scan()
    app = QtWidgets.QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    app.setApplicationName("luma")
    # На Wayland это значение становится app-id, по которому Niri применяет
    # правило `match app-id="^luma$"` из config.kdl.
    app.setDesktopFileName("luma")

    start_watcher()

    # applications может быть экземпляром backend.apps.models.Applications.
    # По умолчанию плашки берутся из configure_badges в frontend/buttons.py;
    # badges позволяет при необходимости передать их напрямую.
    # При обычном запуске список получаем автоматически из desktop-файлов.
    if applications is None:
        applications = apps_obj.get_apps()

    window = MainWindow(
        applications=applications,
        badges=badges,
    )
    if "--background" not in sys.argv:
        window.show()

    toggle_requested = False

    def request_toggle(_signum, _frame):
        nonlocal toggle_requested
        toggle_requested = True

    def process_toggle():
        nonlocal toggle_requested
        if toggle_requested:
            toggle_requested = False
            window.toggle_visibility()

    signal.signal(signal.SIGUSR1, request_toggle)
    toggle_timer = QtCore.QTimer()
    toggle_timer.timeout.connect(process_toggle)
    toggle_timer.start(25)

    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
