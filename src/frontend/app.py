import signal
import sys
from pathlib import Path

from PySide6 import QtCore, QtWidgets

# Сохраняет запуск как `python frontend/app.py` и одновременно позволяет
# backend импортировать `frontend.app` как обычный пакет.
PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from frontend.window import MainWindow


def main(applications=None):
    app = QtWidgets.QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    app.setApplicationName("luma")
    # На Wayland это значение становится app-id, по которому Niri применяет
    # правило `match app-id="^luma$"` из config.kdl.
    app.setDesktopFileName("luma")

    # applications может быть экземпляром backend.apps.models.Applications.
    # При обычном запуске список получаем автоматически из desktop-файлов.
    if applications is None:
        from backend.apps.application_search import get_apps

        applications = get_apps()

    window = MainWindow(applications=applications)
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
