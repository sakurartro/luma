import numpy as np 
import numpy.typing as npt
import onnxruntime as ort 
from tokenizers import Tokenizer
from backend.files.clip_pre_work import pre_image

class Clip:
    def __init__(self):
        #paths
        self.text_model_path = "models/clip/onnx/text_model_int8.onnx"
        self.visual_model_path = "models/clip/onnx/vision_model_int8.onnx"
        self.tokenizer_path = "models/clip/tokenizer.json"

        #sessions
        self.text_session = ort.InferenceSession(
            self.text_model_path,
            providers=["CPUExecutionProvider"]
        )
        self.visual_session = ort.InferenceSession(
            self.visual_model_path,
            providers=["CPUExecutionProvider"]
        )

        #tokenizer
        self.tokenizer = Tokenizer.from_file(self.tokenizer_path)
        self.tokenizer.enable_padding(length=64)
        self.tokenizer.enable_truncation(max_length=64)
        #tensors (text)
        self.text_input = self.text_session.get_inputs()[0].name
        self.text_output = self.text_session.get_outputs()[0].name

        #tensors (vision)
        self.visual_input = self.visual_session.get_inputs()[0].name
        self.visual_output = self.visual_session.get_outputs()[0].name


    def p_text(self, inputs) -> npt.NDArray[np.float32]:
        embeddings = self.tokenizer.encode_batch(inputs)
        embeddings = np.array(
            [e.ids for e in embeddings],
            dtype=np.int64
        )
        result = self.text_session.run(
            [self.text_output],
            {self.text_input: embeddings}
        )[0]
        return result / np.linalg.norm(result, axis=-1, keepdims=True)

    def p_image(self, path: str) -> np.ndarray:
        inputs = pre_image(path)
        result = self.visual_session.run(
            [self.visual_output],
            {self.visual_input: inputs}
        )[0]
        return result / np.linalg.norm(result, axis=-1, keepdims=True)

    def calc_matching(self, text_vec: np.ndarray, img_vec: np.ndarray):
        _match = text_vec @ img_vec.T
        return _match

    

clip = Clip()

if __name__ == "__main__":
    img_data = clip.p_image("/home/sakura/Downloads/richard-brutyo-Sg3XwuEpybU-unsplash.jpg")
    descriptions: list[str] = [
        "Golden retriever sits outdoors holding a yellow rose gently.",
        "Snowy mountains reflect sunlight above a quiet alpine lake."
    ]
    text_data = clip.p_text(descriptions)
    print(clip.calc_matching(text_data, img_data))