import math
from pathlib import Path

from PySide6 import QtCore, QtGui, QtWidgets
from backend.apps.models import Application

from frontend.settings import (
    APPLICATION_COLUMNS,
    APPLICATION_MAX_VISIBLE_ROWS,
    APPLICATION_TILE_SIZE,
)
from frontend.badges import BadgesBar
from frontend.widgets import LiquidGlassFrame


def _application_name(application: Application):
    return application.name


def _fallback_icon(name, size=40):
    """Создаёт простую иконку с первой буквой, если icon_path пустой."""
    scale = 2
    pixmap = QtGui.QPixmap(size * scale, size * scale)
    pixmap.setDevicePixelRatio(scale)
    pixmap.fill(QtCore.Qt.transparent)

    painter = QtGui.QPainter(pixmap)
    painter.setRenderHint(QtGui.QPainter.Antialiasing)
    painter.setPen(QtCore.Qt.NoPen)
    painter.setBrush(QtGui.QColor(93, 112, 126, 210))
    painter.drawRoundedRect(QtCore.QRectF(1, 1, size - 2, size - 2), 10, 10)
    painter.setPen(QtGui.QColor(239, 244, 247))
    font = painter.font()
    font.setPixelSize(17)
    font.setWeight(QtGui.QFont.Medium)
    painter.setFont(font)
    painter.drawText(QtCore.QRectF(0, 0, size, size), QtCore.Qt.AlignCenter, name[:1].upper())
    painter.end()
    return QtGui.QIcon(pixmap)


def _application_icon(application: Application, name: str):
    icon_path = application.icon_path
    if icon_path and Path(icon_path).is_file():
        icon = QtGui.QIcon(str(icon_path))
        if not icon.isNull():
            return icon

    return _fallback_icon(name)


class ApplicationTile(QtWidgets.QToolButton):
    """Одна плитка, соответствующая одному Application из backend."""

    selected = QtCore.Signal(object)

    def __init__(self, application: Application, parent=None):
        super().__init__(parent)
        self.setObjectName("applicationTile")
        self.setFixedSize(*APPLICATION_TILE_SIZE)
        self.setToolButtonStyle(QtCore.Qt.ToolButtonTextUnderIcon)
        self.setIconSize(QtCore.QSize(40, 40))
        self.setCursor(QtCore.Qt.PointingHandCursor)
        self.clicked.connect(lambda: self.selected.emit(self.application))
        self.application = application
        self.update_application(application, force=True)

    def update_application(self, application: Application, force: bool = False):
        previous = self.application
        self.application = application
        if force or (previous.name, previous.icon_path) != (
            application.name,
            application.icon_path,
        ):
            name = _application_name(application)
            self.setIcon(_application_icon(application, name))
            self.setText(name)
            self.setToolTip(name)


class ApplicationsPanel(LiquidGlassFrame):
    """Сетка приложений из списка объектов Application."""

    application_selected = QtCore.Signal(object)
    content_height_changed = QtCore.Signal()

    def __init__(self, parent=None):
        super().__init__(radius=20, surface=False, parent=parent)

        root = QtWidgets.QVBoxLayout(self)
        root.setContentsMargins(10, 10, 8, 10)
        root.setSpacing(8)

        self.badges_bar = BadgesBar()
        self.badges_bar.badge_selected.connect(self._show_badge)
        root.addWidget(self.badges_bar)

        self.scroll = QtWidgets.QScrollArea()
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(QtCore.Qt.ScrollBarAlwaysOff)

        self.content = QtWidgets.QWidget()
        self.grid = QtWidgets.QGridLayout(self.content)
        self.grid.setContentsMargins(0, 0, 0, 0)
        self.grid.setHorizontalSpacing(8)
        self.grid.setVerticalSpacing(8)
        self.grid.setAlignment(QtCore.Qt.AlignTop | QtCore.Qt.AlignLeft)
        self._tiles: dict[str, ApplicationTile] = {}
        self.scroll.setWidget(self.content)

        self.empty_label = QtWidgets.QLabel("Нет приложений")
        self.empty_label.setObjectName("emptyApplications")
        self.empty_label.setAlignment(QtCore.Qt.AlignCenter)

        root.addWidget(self.scroll)
        root.addWidget(self.empty_label)
        self.set_items([])

    def set_applications(self, applications: list[Application]):
        """Принимает список объектов Application."""
        self.badges_bar.clear_selection()
        self.set_items(applications)

    def set_badges(self, badges):
        """Создаёт плашки из ``{name: [Application, ...]}``."""
        models = self.badges_bar.set_badges(badges)
        self.setFixedHeight(self.preferred_height())
        self.content_height_changed.emit()
        return models

    @QtCore.Slot(object)
    def _show_badge(self, badge):
        self.set_items(badge.apps)
        self.content_height_changed.emit()

    def set_items(self, items: list[Application]):
        while self.grid.count():
            layout_item = self.grid.takeAt(0)
            if layout_item.widget() is not None:
                layout_item.widget().hide()

        for index, application in enumerate(items):
            tile = self._tiles.get(application.app_path)
            if tile is None:
                tile = ApplicationTile(application)
                tile.selected.connect(self.application_selected)
                self._tiles[application.app_path] = tile
            else:
                tile.update_application(application)
            row, column = divmod(index, APPLICATION_COLUMNS)
            self.grid.addWidget(tile, row, column)
            tile.show()

        self.item_count = len(items)
        self.scroll.setVisible(bool(items))
        self.empty_label.setVisible(not items)
        self.setFixedHeight(self.preferred_height())

    def preferred_height(self):
        badges_height = 36 if not self.badges_bar.isHidden() else 0
        if self.item_count == 0:
            return 76 + badges_height

        rows = math.ceil(self.item_count / APPLICATION_COLUMNS)
        visible_rows = min(rows, APPLICATION_MAX_VISIBLE_ROWS)
        tile_height = APPLICATION_TILE_SIZE[1]
        return (
            20
            + badges_height
            + visible_rows * tile_height
            + (visible_rows - 1) * 8
        )
