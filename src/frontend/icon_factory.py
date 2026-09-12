from PySide6 import QtCore, QtGui

from frontend.settings import GLASS_ICON_COLOR


def make_icon(name: str, size: int = 24, color: str = GLASS_ICON_COLOR):
    """Создаёт лёгкую векторную иконку без внешних ресурсов."""
    scale = 2
    pixmap = QtGui.QPixmap(size * scale, size * scale)
    pixmap.setDevicePixelRatio(scale)
    pixmap.fill(QtCore.Qt.transparent)

    painter = QtGui.QPainter(pixmap)
    painter.setRenderHint(QtGui.QPainter.Antialiasing)
    painter.scale(size / 24, size / 24)

    pen = QtGui.QPen(QtGui.QColor(color), 1.55)
    pen.setCapStyle(QtCore.Qt.RoundCap)
    pen.setJoinStyle(QtCore.Qt.RoundJoin)
    painter.setPen(pen)
    painter.setBrush(QtCore.Qt.NoBrush)

    if name == "search":
        painter.drawEllipse(QtCore.QRectF(4.8, 4.8, 10.2, 10.2))
        painter.drawLine(QtCore.QPointF(13.5, 13.5), QtCore.QPointF(19.0, 19.0))
    elif name == "applications":
        for x in (5.0, 13.0):
            for y in (5.0, 13.0):
                painter.drawRoundedRect(QtCore.QRectF(x, y, 6.0, 6.0), 1.5, 1.5)
    elif name == "folder":
        path = QtGui.QPainterPath()
        path.moveTo(3.2, 7.2)
        path.lineTo(9.0, 7.2)
        path.lineTo(11.1, 9.2)
        path.lineTo(20.8, 9.2)
        path.lineTo(20.8, 18.8)
        path.quadTo(20.8, 20.2, 19.3, 20.2)
        path.lineTo(4.7, 20.2)
        path.quadTo(3.2, 20.2, 3.2, 18.7)
        path.closeSubpath()
        painter.drawPath(path)
        painter.drawLine(QtCore.QPointF(3.5, 10.1), QtCore.QPointF(20.4, 10.1))
    elif name == "layers":
        top = QtGui.QPolygonF(
            [
                QtCore.QPointF(12, 3.8),
                QtCore.QPointF(20.2, 8.2),
                QtCore.QPointF(12, 12.7),
                QtCore.QPointF(3.8, 8.2),
            ]
        )
        painter.drawPolygon(top)
        painter.drawPolyline(
            [QtCore.QPointF(4.1, 12.2), QtCore.QPointF(12, 16.5), QtCore.QPointF(19.9, 12.2)]
        )
        painter.drawPolyline(
            [QtCore.QPointF(4.1, 16.2), QtCore.QPointF(12, 20.5), QtCore.QPointF(19.9, 16.2)]
        )
    elif name == "documents":
        painter.drawRoundedRect(QtCore.QRectF(8.0, 4.0, 10.5, 13.5), 1.4, 1.4)
        painter.drawRoundedRect(QtCore.QRectF(5.0, 7.0, 10.5, 13.5), 1.4, 1.4)
    elif name == "close":
        painter.drawLine(QtCore.QPointF(7.5, 7.5), QtCore.QPointF(16.5, 16.5))
        painter.drawLine(QtCore.QPointF(16.5, 7.5), QtCore.QPointF(7.5, 16.5))

    painter.end()
    return QtGui.QIcon(pixmap)
