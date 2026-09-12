import sys
from pathlib import Path

from PySide6 import QtWidgets

# Сохраняет запуск как `python frontend/app.py` и одновременно позволяет
# backend импортировать `frontend.app` как обычный пакет.
PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from frontend.window import MainWindow


def main(applications=None):
    app = QtWidgets.QApplication(sys.argv)
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
    window.show()

    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
