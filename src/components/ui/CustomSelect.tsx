import React, { useState, useRef, useEffect } from 'react';
import { ChevronDown, Check } from 'lucide-react';

export interface SelectOption {
    id: string;
    label: string;
    value: string;
    icon?: React.ReactNode;
}

interface CustomSelectProps {
    value: string;
    onChange: (value: string) => void;
    options: SelectOption[];
    placeholder?: string;
    disabled?: boolean;
    className?: string;
}

export function CustomSelect({
    value,
    onChange,
    options,
    placeholder = "Select...",
    disabled = false,
    className = ""
}: CustomSelectProps) {
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    const selectedOption = options.find(opt => opt.value === value);

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
                setIsOpen(false);
            }
        };

        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    const handleSelect = (optionValue: string) => {
        onChange(optionValue);
        setIsOpen(false);
    };

    return (
        <div ref={containerRef} className={`relative ${className}`}>
            <button
                type="button"
                onClick={() => !disabled && setIsOpen(!isOpen)}
                disabled={disabled}
                className={`
                    w-full flex items-center justify-between gap-2 px-3 py-2.5 rounded-lg border text-sm font-medium transition-all
                    ${disabled
                        ? "opacity-50 cursor-not-allowed bg-[var(--color-bg-secondary)] border-[var(--color-border-secondary)] text-[var(--color-text-tertiary)]"
                        : isOpen
                            ? "bg-[var(--color-bg-primary)] border-blue-500 ring-1 ring-blue-500 text-[var(--color-text-primary)]"
                            : "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)] text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
                    }
                `}
            >
                <div className="flex items-center gap-2 truncate">
                    {selectedOption?.icon && <span className="opacity-70">{selectedOption.icon}</span>}
                    <span className={!selectedOption ? "text-[var(--color-text-tertiary)]" : ""}>
                        {selectedOption ? selectedOption.label : placeholder}
                    </span>
                </div>
                <ChevronDown size={16} className={`text-[var(--color-text-tertiary)] transition-transform ${isOpen ? "rotate-180" : ""}`} />
            </button>

            {isOpen && (
                <div className="absolute top-full left-0 right-0 mt-1 max-h-60 overflow-y-auto rounded-lg border border-[var(--color-border-primary)] bg-[var(--color-bg-elevated)] shadow-xl z-50 py-1 animate-in fade-in zoom-in-95 duration-100">
                    {options.length > 0 ? (
                        options.map((option) => (
                            <button
                                key={option.id}
                                onClick={() => handleSelect(option.value)}
                                className={`
                                    w-full px-3 py-2 text-sm text-left flex items-center gap-2 transition-colors
                                    ${option.value === value
                                        ? "bg-blue-600/10 text-blue-500 font-semibold"
                                        : "text-[var(--color-text-primary)] hover:bg-[var(--color-hover-bg)]"
                                    }
                                `}
                            >
                                {option.icon && <span className="opacity-70 w-4 h-4 flex items-center justify-center">{option.icon}</span>}
                                <span className="flex-1 truncate">{option.label}</span>
                                {option.value === value && <Check size={14} className="flex-shrink-0" />}
                            </button>
                        ))
                    ) : (
                        <div className="px-3 py-2 text-xs text-[var(--color-text-tertiary)] text-center italic">
                            No options available
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
