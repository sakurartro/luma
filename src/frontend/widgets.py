from PySide6 import QtCore, QtGui, QtWidgets

from frontend.icon_factory import make_icon


def add_shadow(widget, blur=48, y_offset=14, alpha=72):
    shadow = QtWidgets.QGraphicsDropShadowEffect(widget)
    shadow.setBlurRadius(blur)
    shadow.setOffset(0, y_offset)
    shadow.setColor(QtGui.QColor(0, 0, 0, alpha))
    widget.setGraphicsEffect(shadow)


class LiquidGlassFrame(QtWidgets.QFrame):
    """Плотная полупрозрачная системная панель с тонкой кромкой."""

    def __init__(self, radius=22, surface=True, parent=None):
        super().__init__(parent)
        self.radius = radius
        self.surface = surface
        self.setAttribute(QtCore.Qt.WA_TranslucentBackground)

    def paintEvent(self, event):
        if not self.surface:
            super().paintEvent(event)
            return

        painter = QtGui.QPainter(self)
        painter.setRenderHint(QtGui.QPainter.Antialiasing)
        rect = QtCore.QRectF(self.rect()).adjusted(0.7, 0.7, -0.7, -0.7)
        path = QtGui.QPainterPath()
        path.addRoundedRect(rect, self.radius, self.radius)

        painter.fillPath(path, QtGui.QColor(20, 22, 28, 230))
        painter.setPen(QtGui.QPen(QtGui.QColor(255, 255, 255, 20), 1.0))
        painter.drawPath(path)
        painter.end()


class SearchBox(QtWidgets.QFrame):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setFixedHeight(40)

        layout = QtWidgets.QHBoxLayout(self)
        layout.setContentsMargins(6, 0, 8, 0)
        layout.setSpacing(10)

        icon = QtWidgets.QLabel()
        icon.setPixmap(make_icon("search", 20, "#a6f4f4f5").pixmap(20, 20))

        self.input = QtWidgets.QLineEdit()
        self.input.setPlaceholderText("Spotlight Search")
        self.input.setClearButtonEnabled(True)

        layout.addWidget(icon)
        layout.addWidget(self.input, 1)


class GlassIconButton(QtWidgets.QPushButton):
    def __init__(self, icon_name, tooltip, size=(42, 42), active=False, parent=None):
        super().__init__(parent)
        self.setObjectName("glassButton")
        self.setProperty("active", active)
        self.setFixedSize(*size)
        icon_color = "#fff4f4f5" if active else "#a6f4f4f5"
        self.setIcon(make_icon(icon_name, 21, icon_color))
        self.setIconSize(QtCore.QSize(21, 21))
        self.setToolTip(tooltip)
        self.setCursor(QtCore.Qt.PointingHandCursor)
