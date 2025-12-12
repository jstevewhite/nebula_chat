
import { Brain, X } from "lucide-react";

interface MemoryPanelProps {
    memories: string[];
    onClose: () => void;
}

export default function MemoryPanel({ memories, onClose }: MemoryPanelProps) {
    return (
        <div className="w-80 h-full border-l border-gray-800 bg-gray-900 flex flex-col shadow-xl absolute right-0 top-0 z-20 animate-in slide-in-from-right duration-200">
            <div className="p-4 border-b border-gray-800 flex justify-between items-center bg-gray-900/50 backdrop-blur">
                <h3 className="text-sm font-semibold text-gray-200 flex items-center gap-2">
                    <Brain size={16} className="text-purple-400" />
                    Memory Context
                </h3>
                <button
                    onClick={onClose}
                    className="text-gray-500 hover:text-white transition-colors p-1 hover:bg-gray-800 rounded"
                >
                    <X size={16} />
                </button>
            </div>

            <div className="flex-1 overflow-y-auto p-4 space-y-3">
                {memories.length === 0 ? (
                    <div className="text-center text-gray-500 mt-10 text-sm italic">
                        No active memories found for this interaction.
                    </div>
                ) : (
                    memories.map((mem, i) => (
                        <div key={i} className="bg-gray-800/50 border border-gray-700/50 p-3 rounded-lg text-sm text-gray-300 shadow-sm hover:border-purple-500/30 transition-colors">
                            <div className="text-xs text-purple-400 mb-1 font-mono opacity-75">MEMORY FLAGMENT {i + 1}</div>
                            "{mem}"
                        </div>
                    ))
                )}
            </div>
        </div>
    );
}
