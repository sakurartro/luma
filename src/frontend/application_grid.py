import math
from pathlib import Path

from PySide6 import QtCore, QtGui, QtWidgets

from frontend.settings import (
    APPLICATION_COLUMNS,
    APPLICATION_MAX_VISIBLE_ROWS,
    APPLICATION_TILE_SIZE,
)
from frontend.widgets import LiquidGlassFrame


def _application_name(application):
    return str(getattr(application, "name", application.__class__.__name__))


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


def _application_icon(application, name):
    icon_value = getattr(application, "icon", None)
    if isinstance(icon_value, QtGui.QIcon) and not icon_value.isNull():
        return icon_value
    if isinstance(icon_value, QtGui.QPixmap) and not icon_value.isNull():
        return QtGui.QIcon(icon_value)

    icon_path = getattr(application, "icon_path", None)
    if icon_path and Path(icon_path).is_file():
        icon = QtGui.QIcon(str(icon_path))
        if not icon.isNull():
            return icon

    return _fallback_icon(name)


class ApplicationTile(QtWidgets.QToolButton):
    """Одна плитка, соответствующая одному Application из backend."""

    selected = QtCore.Signal(object)

    def __init__(self, application, parent=None):
        super().__init__(parent)
        name = _application_name(application)

        self.application = application
        self.setObjectName("applicationTile")
        self.setFixedSize(*APPLICATION_TILE_SIZE)
        self.setToolButtonStyle(QtCore.Qt.ToolButtonTextUnderIcon)
        self.setIcon(_application_icon(application, name))
        self.setIconSize(QtCore.QSize(40, 40))
        self.setText(name)
        self.setToolTip(name)
        self.setCursor(QtCore.Qt.PointingHandCursor)
        self.clicked.connect(lambda: self.selected.emit(self.application))


class ApplicationsPanel(LiquidGlassFrame):
    """Сетка, которая принимает Applications или обычный список объектов."""

    application_selected = QtCore.Signal(object)

    def __init__(self, parent=None):
        super().__init__(radius=20, surface=False, parent=parent)

        root = QtWidgets.QVBoxLayout(self)
        root.setContentsMargins(10, 10, 8, 10)

        self.scroll = QtWidgets.QScrollArea()
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(QtCore.Qt.ScrollBarAlwaysOff)

        self.content = QtWidgets.QWidget()
        self.grid = QtWidgets.QGridLayout(self.content)
        self.grid.setContentsMargins(0, 0, 0, 0)
        self.grid.setHorizontalSpacing(8)
        self.grid.setVerticalSpacing(8)
        self.grid.setAlignment(QtCore.Qt.AlignTop | QtCore.Qt.AlignLeft)
        self.scroll.setWidget(self.content)

        self.empty_label = QtWidgets.QLabel("Нет приложений")
        self.empty_label.setObjectName("emptyApplications")
        self.empty_label.setAlignment(QtCore.Qt.AlignCenter)

        root.addWidget(self.scroll)
        root.addWidget(self.empty_label)
        self.set_items([])

    def set_applications(self, applications):
        """Принимает backend.apps.models.Applications или обычный список."""
        items = getattr(applications, "apps", applications)
        self.set_items(list(items or []))

    def set_items(self, items):
        while self.grid.count():
            layout_item = self.grid.takeAt(0)
            if layout_item.widget() is not None:
                layout_item.widget().deleteLater()

        for index, application in enumerate(items):
            tile = ApplicationTile(application)
            tile.selected.connect(self.application_selected)
            row, column = divmod(index, APPLICATION_COLUMNS)
            self.grid.addWidget(tile, row, column)

        self.item_count = len(items)
        self.scroll.setVisible(bool(items))
        self.empty_label.setVisible(not items)
        self.setFixedHeight(self.preferred_height())

    def preferred_height(self):
        if self.item_count == 0:
            return 76

        rows = math.ceil(self.item_count / APPLICATION_COLUMNS)
        visible_rows = min(rows, APPLICATION_MAX_VISIBLE_ROWS)
        tile_height = APPLICATION_TILE_SIZE[1]
        return 20 + visible_rows * tile_height + (visible_rows - 1) * 8
